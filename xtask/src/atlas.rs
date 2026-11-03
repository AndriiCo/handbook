//! `cargo xtask atlas` — run Atlas task packs (spec
//! `docs/specs/atlas-harness.md`).
//!
//! Stub port by default (offline-safe); `--live` calls the Meta
//! Model API with the sealed `meta.api_key` (or `$META_API_KEY`).

use std::path::PathBuf;
use std::time::Duration;

use atlas::batch::{BatchJob, BatchRunner, job_dir};
use atlas::cached::CachedTransport;
use atlas::config::{AtlasConfig, Effort, Protocol};
use atlas::model::{BASE_URL, MetaClient, ModelPort, StubPort, UreqTransport};

/// `atlas --config <pack> --task <name> [--effort …] [--model …]
/// [--protocol …] [--out …] [--refresh] [--limit N] [--live]
/// [--dry-run] [--run-id …] [--pdf-tool <argv…>]`, plus
/// `--batch-poll <job>` and `--check`.
pub fn run(args: &[String]) {
    if args.iter().any(|a| a == "--check") {
        return run_check(args);
    }
    if let Some(job) = super::flag(args, "batch-poll") {
        return run_batch_poll(args, &job);
    }
    let config_path = super::required(args, "config");
    let task_name = super::required(args, "task");
    let path = PathBuf::from(&config_path);
    let (cfg, pack_dir) = AtlasConfig::load(&path).unwrap_or_else(|e| {
        eprintln!("atlas: {e}");
        std::process::exit(2);
    });
    let options = atlas::task::RunOptions {
        effort: super::flag(args, "effort").and_then(|v| {
            Effort::parse(&v).or_else(|| {
                eprintln!("atlas: --effort must be low|standard|high|max");
                std::process::exit(2);
            })
        }),
        model: super::flag(args, "model"),
        protocol: super::flag(args, "protocol").and_then(|v| {
            Protocol::parse(&v).or_else(|| {
                eprintln!("atlas: --protocol must be fast|batch|auto");
                std::process::exit(2);
            })
        }),
        out: super::flag(args, "out").map(PathBuf::from),
        refresh: args.iter().any(|a| a == "--refresh"),
        limit: super::flag(args, "limit").and_then(|v| v.parse().ok()),
        dry_run: args.iter().any(|a| a == "--dry-run"),
        pdf_tool: pdf_tool(args),
        // Every live source fetch is archived; replay-only reruns offline.
        fetch_cache: Some(crate::call_cache::fetch_cache(args)),
    };
    let live = args.iter().any(|a| a == "--live");
    let out = options
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from(&cfg.run.out));
    let run_id =
        super::flag(args, "run-id").unwrap_or_else(|| format!("run-{}", super::unix_now()));
    if live {
        let client = live_client(args);
        let runner =
            atlas::task::Runner::new(&client, &cfg, &pack_dir, out, &run_id, super::unix_now());
        report(runner.run(&task_name, &options));
    } else {
        let port = StubPort::new("{}", "{}");
        let runner =
            atlas::task::Runner::new(&port, &cfg, &pack_dir, out, &run_id, super::unix_now());
        report(runner.run(&task_name, &options));
    }
}

fn report(outcome: Result<atlas::task::TaskOutcome, atlas::error::Error>) {
    match outcome {
        Ok(outcome) => {
            let plan = &outcome.plan;
            println!(
                "task: {}\nmodel: {} ({:?})\neffort: {:?}\nprotocol: {:?}\ndocs: {} (cache-skipped {})\nchunks: {}\nest input tokens: {}\ncalls: {} batch: {}\nnative blocks: {} uploads: {}\nest cost: ${:.4}",
                plan.task,
                plan.model,
                plan.mode,
                plan.effort,
                plan.protocol,
                plan.docs,
                plan.skipped_cache,
                plan.chunks,
                plan.est_input_tokens,
                plan.calls,
                plan.batch,
                plan.native_blocks,
                plan.uploads,
                plan.est_cost_usd,
            );
            for file in &outcome.out_files {
                println!("wrote: {}", file.display());
            }
        }
        Err(e) => {
            eprintln!("atlas: {e}");
            std::process::exit(1);
        }
    }
}

/// `--pdf-tool` consumes the rest of argv as the extractor command.
fn pdf_tool(args: &[String]) -> Option<Vec<String>> {
    let pos = args.iter().position(|a| a == "--pdf-tool")?;
    let argv: Vec<String> = args[pos + 1..].to_vec();
    if argv.is_empty() {
        eprintln!("atlas: --pdf-tool needs a command");
        std::process::exit(2);
    }
    Some(argv)
}

/// The live atlas client: every call goes through the call cache.
pub(crate) type LiveClient = MetaClient<CachedTransport<UreqTransport>>;

/// Record/replay transport for this invocation (`--meta-cache`,
/// `--cache-mode`, `--daily-call-cap`; see `call_cache`).
fn cached_transport(args: &[String]) -> CachedTransport<UreqTransport> {
    CachedTransport::new(
        UreqTransport::default(),
        crate::call_cache::meta_cache(args),
    )
}

/// Live client: `$META_API_KEY` first, sealed `meta.api_key` fallback.
/// Shared with the instruct worker (same key, same rules). Repeat
/// requests replay from the call cache and only live calls count
/// toward `meta.daily_call_cap`.
pub(crate) fn live_client(args: &[String]) -> LiveClient {
    if let Ok(key) = std::env::var("META_API_KEY")
        && !key.is_empty()
    {
        eprintln!("atlas: using $META_API_KEY");
        return MetaClient::new(cached_transport(args), BASE_URL, key);
    }
    let store = super::open_store(args);
    let conn = super::open_db(&store);
    let (kek, _source) = sealed_config::load_kek().unwrap_or_else(|e| {
        eprintln!("atlas: {e}");
        std::process::exit(1);
    });
    let (cfg, warnings) = sealed_config::RuntimeConfig::load(&conn, &kek).unwrap_or_else(|e| {
        eprintln!("atlas: config: {e}");
        std::process::exit(1);
    });
    for warning in &warnings {
        eprintln!("atlas: config warning: {warning}");
    }
    let Some(key) = cfg.meta_api_key else {
        eprintln!("atlas: meta.api_key is not set (admin config UI) and $META_API_KEY is empty");
        std::process::exit(2);
    };
    MetaClient::new(
        cached_transport(args),
        &cfg.meta_base_url,
        key.as_str().to_string(),
    )
}

/// `--check`: verify the key against `GET /models`.
fn run_check(args: &[String]) {
    let client = live_client(args);
    match client.list_models() {
        Ok(ids) => println!("meta api ok: {}", ids.join(", ")),
        Err(e) => {
            eprintln!("atlas --check: {e}");
            std::process::exit(1);
        }
    }
}

/// `--batch-poll <job>`: resume polling a batch job to completion.
fn run_batch_poll(args: &[String], job: &str) {
    let config_path = super::required(args, "config");
    let (cfg, _) = AtlasConfig::load(&PathBuf::from(&config_path)).unwrap_or_else(|e| {
        eprintln!("atlas: {e}");
        std::process::exit(2);
    });
    // The model (for RPM pacing) comes from the job itself.
    let out = super::flag(args, "out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&cfg.run.out));
    // Jobs live under the run dir; accept `<run>/<job>` or bare
    // `<job>` resolved against the newest run dir.
    let dir = match job.split_once('/') {
        Some((run, rest)) => job_dir(&out.join(run), rest),
        None => find_job_dir(&out, job).unwrap_or_else(|| {
            eprintln!("atlas: batch job not found: {job}");
            std::process::exit(2);
        }),
    };
    let mut batch = BatchJob::open(&dir).unwrap_or_else(|e| {
        eprintln!("atlas: {e}");
        std::process::exit(1);
    });
    let rpm = cfg
        .models
        .iter()
        .find(|m| m.id == batch.model)
        .map(|m| m.rpm)
        .unwrap_or(60);
    let rounds: u32 = super::flag(args, "rounds")
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let client = live_client(args);
    let runner = BatchRunner::new(&client, rpm.max(1), Duration::from_secs(5));
    if let Err(e) = runner.submit_pending(&mut batch, &dir) {
        eprintln!("atlas: submit: {e}");
        std::process::exit(1);
    }
    match runner.poll_until_done(&mut batch, &dir, rounds) {
        Ok((done, pending, failed)) => {
            println!("job: {job}\ndone: {done}\npending: {pending}\nfailed: {failed}");
            if pending > 0 {
                println!("resume: cargo xtask atlas --config {config_path} --batch-poll {job}");
            }
        }
        Err(e) => {
            eprintln!("atlas: poll: {e}");
            std::process::exit(1);
        }
    }
}

/// Find `out/<run>/batch/<job>/job.json`, newest run first.
fn find_job_dir(out: &std::path::Path, job: &str) -> Option<PathBuf> {
    let mut runs: Vec<PathBuf> = std::fs::read_dir(out)
        .ok()?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_dir())
        .collect();
    runs.sort();
    runs.into_iter().rev().find_map(|run| {
        let dir = job_dir(&run, job);
        dir.join("job.json").is_file().then_some(dir)
    })
}
