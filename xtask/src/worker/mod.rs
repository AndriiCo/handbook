//! `cargo xtask worker` — plant worker for the production work queue
//! (P392, spec `docs/specs/work-queue.md`).
//!
//! ```text
//! cargo xtask worker --connect wss://andrii.ai/apostille/api/v1/worker/ws
//!     [--name plant] [--workdir /srv/scratch/apostille/work]
//!     [--cache-dir /srv/scratch/apostille] [--env-file ~/.config/apostille/worker.env]
//!     [--gpu-lock ~/apostille-pipeline/gpu.lock] [--once]
//! cargo xtask worker --run-local <job.json> [--workdir …]   # one job, no server
//! cargo xtask worker --unpack <bundle.json> [--into packs/world-nonparty]
//! cargo xtask worker --kinds
//! ```
//!
//! Dials OUT to production, runs one job at a time, reconnects with
//! backoff, and holds `<workdir>/worker.lock` so a cron watchdog can
//! start it every minute (the second copy exits quietly). GPU kinds
//! also hold `~/apostille-pipeline/gpu.lock` while they run.
//!
//! New kinds are one file each: implement [`JobHandler`] and add the
//! handler to [`handlers`] (and the server's `KINDS` list in
//! `crates/web/src/worker.rs`).

mod client;
mod eval_gold;
mod precheck_glimmer;
pub(crate) mod research_nonparty;
mod scrape_urls;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::flag;

/// Default scratch layout on plant (operator decision 2026-11-03).
const DEFAULT_WORKDIR: &str = "/srv/scratch/apostille/work";
/// Token + Meta key file (0600, never in the repo).
const DEFAULT_ENV_FILE: &str = ".config/apostille/worker.env";
/// Shared with the plant build pipeline's GPU stages.
const DEFAULT_GPU_LOCK: &str = "apostille-pipeline/gpu.lock";

/// One job's environment: where to write, where the repo is, secrets.
pub struct JobContext {
    pub id: i64,
    /// Per-attempt scratch dir (`<workdir>/job-<id>-a<attempt>`).
    pub dir: PathBuf,
    /// Repository checkout (packs, fixtures, gold sets).
    pub repo: PathBuf,
    /// Cross-job caches (`<cache-dir>/fetch-cache`, …).
    pub cache_dir: PathBuf,
    pub env: WorkerEnv,
}

/// What a finished job uploads.
#[derive(Debug, Clone, PartialEq)]
pub struct JobOutput {
    pub content_type: String,
    pub bytes: Vec<u8>,
    /// Small JSON the admin list shows (counts, not content).
    pub summary: Value,
}

impl JobOutput {
    /// Pretty JSON result + summary.
    pub fn json(value: &Value, summary: Value) -> Self {
        Self {
            content_type: "application/json".to_string(),
            bytes: serde_json::to_vec_pretty(value).expect("json serializes"),
            summary,
        }
    }
}

/// Why a job failed; `retryable` lets the queue try again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobError {
    pub message: String,
    pub retryable: bool,
}

impl JobError {
    /// Bad payload or a missing prerequisite: retrying cannot help.
    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }

    /// Network or model trouble: the next attempt may succeed.
    pub fn retry(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }
}

/// One job kind. Implementations live one per file in this module.
pub trait JobHandler: Send + Sync {
    /// Queue kind (`research.nonparty`, …).
    fn kind(&self) -> &'static str;
    /// GPU kinds run under `gpu.lock`.
    fn gpu(&self) -> bool {
        false
    }
    /// False keeps the kind out of `hello` capabilities, so its jobs
    /// stay pending until a worker that can run them connects. `env` is
    /// `worker.env` (falling back to the process environment).
    fn available(&self, _env: &WorkerEnv) -> bool {
        true
    }
    fn run(&self, ctx: &JobContext, payload: &Value) -> Result<JobOutput, JobError>;
}

/// Every handler this worker knows.
pub fn handlers() -> Vec<Box<dyn JobHandler>> {
    vec![
        Box::new(research_nonparty::ResearchNonparty),
        Box::new(scrape_urls::ScrapeUrls),
        Box::new(eval_gold::EvalGold),
        Box::new(precheck_glimmer::PrecheckGlimmer),
    ]
}

/// Values from `worker.env` (`KEY=VALUE` lines; `#` comments and an
/// `export ` prefix are allowed). Debug never prints values.
#[derive(Clone, Default)]
pub struct WorkerEnv {
    values: HashMap<String, String>,
}

impl std::fmt::Debug for WorkerEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut keys: Vec<&String> = self.values.keys().collect();
        keys.sort();
        f.debug_struct("WorkerEnv").field("keys", &keys).finish()
    }
}

impl WorkerEnv {
    pub fn parse(text: &str) -> Self {
        let mut values = HashMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            values.insert(key.trim().to_string(), value.to_string());
        }
        Self { values }
    }

    /// File value, else the process environment (empty = unset).
    pub fn get(&self, key: &str) -> Option<String> {
        self.values
            .get(key)
            .cloned()
            .or_else(|| std::env::var(key).ok())
            .filter(|v| !v.is_empty())
    }

    /// Load `path`, refusing files other users can read (0600 rule,
    /// same as `meta-staging.env`). A missing file is an empty env.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(path)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .permissions()
                .mode();
            if mode & 0o077 != 0 {
                return Err(format!(
                    "{} is readable by other users (mode {:o}); run chmod 600 on it",
                    path.display(),
                    mode & 0o777
                ));
            }
        }
        Ok(Self::parse(&text))
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from)
}

/// Repo root: `$APOSTILLE_REPO`, else the working directory when it
/// holds the workspace, else the checkout this binary was built from.
pub fn repo_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("APOSTILLE_REPO") {
        return PathBuf::from(dir);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if cwd.join(research_nonparty::PACK_PATH).is_file() {
        return cwd;
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or(cwd, Path::to_path_buf)
}

/// Take `path` exclusively without waiting; `None` when another
/// process holds it (the lock lives as long as the returned file).
fn try_lock(path: &Path) -> Result<Option<std::fs::File>, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(format!("{}: {e}", path.display())),
    }
}

/// Run one handler in `ctx`, holding the GPU lock for GPU kinds.
pub fn run_job(
    handler: &dyn JobHandler,
    ctx: &JobContext,
    payload: &Value,
    gpu_lock: &Path,
) -> Result<JobOutput, JobError> {
    std::fs::create_dir_all(&ctx.dir)
        .map_err(|e| JobError::retry(format!("{}: {e}", ctx.dir.display())))?;
    let _gpu = if handler.gpu() {
        if let Some(parent) = gpu_lock.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(gpu_lock)
            .map_err(|e| JobError::retry(format!("{}: {e}", gpu_lock.display())))?;
        eprintln!("worker: job {} waiting for {}", ctx.id, gpu_lock.display());
        file.lock()
            .map_err(|e| JobError::retry(format!("{}: {e}", gpu_lock.display())))?;
        Some(file)
    } else {
        None
    };
    handler.run(ctx, payload)
}

pub fn run(args: &[String]) {
    let env_path =
        flag(args, "env-file").map_or_else(|| home().join(DEFAULT_ENV_FILE), PathBuf::from);
    if args.iter().any(|a| a == "--kinds") {
        let env = WorkerEnv::load(&env_path).unwrap_or_else(|e| {
            eprintln!("worker: {e}");
            WorkerEnv::default()
        });
        for h in handlers() {
            let gpu = if h.gpu() { " (gpu)" } else { "" };
            let ready = if h.available(&env) {
                ""
            } else {
                " [not announced]"
            };
            println!("{}{gpu}{ready}", h.kind());
        }
        return;
    }
    if let Some(bundle) = flag(args, "unpack") {
        let into = flag(args, "into").unwrap_or_else(|| research_nonparty::OUT_DIR.to_string());
        match research_nonparty::unpack(Path::new(&bundle), Path::new(&into)) {
            Ok(n) => println!("worker: wrote {n} pack(s) to {into}"),
            Err(e) => {
                eprintln!("worker --unpack: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let workdir = PathBuf::from(flag(args, "workdir").unwrap_or_else(|| DEFAULT_WORKDIR.into()));
    let cache_dir = flag(args, "cache-dir").map_or_else(
        || {
            workdir
                .parent()
                .map_or_else(|| workdir.clone(), Path::to_path_buf)
        },
        PathBuf::from,
    );
    let gpu_lock =
        flag(args, "gpu-lock").map_or_else(|| home().join(DEFAULT_GPU_LOCK), PathBuf::from);
    let env = WorkerEnv::load(&env_path).unwrap_or_else(|e| {
        eprintln!("worker: {e}");
        std::process::exit(2);
    });
    if let Some(job) = flag(args, "run-local") {
        return run_local(&job, &workdir, &cache_dir, env, &gpu_lock);
    }
    let Some(url) = flag(args, "connect") else {
        eprintln!("worker: --connect <wss url>, --run-local <job.json>, --unpack or --kinds");
        std::process::exit(2);
    };
    let Some(token) = env.get("APOSTILLE_WORKER_TOKEN") else {
        eprintln!(
            "worker: APOSTILLE_WORKER_TOKEN missing ({} or the environment)",
            env_path.display()
        );
        std::process::exit(2);
    };
    let _singleton = match try_lock(&workdir.join("worker.lock")) {
        Ok(Some(file)) => file,
        Ok(None) => {
            eprintln!(
                "worker: already running ({} held)",
                workdir.join("worker.lock").display()
            );
            return;
        }
        Err(e) => {
            eprintln!("worker: {e}");
            std::process::exit(1);
        }
    };
    let cfg = client::Config {
        url,
        token,
        name: flag(args, "name").unwrap_or_else(|| "plant".to_string()),
        workdir,
        cache_dir,
        repo: repo_root(),
        env,
        gpu_lock,
        once: args.iter().any(|a| a == "--once"),
    };
    client::run_with(&cfg, std::sync::Arc::new(handlers()));
}

/// `--run-local <job.json>`: `{kind, payload}` from a file (or `-`
/// for stdin), run here, result written next to the job dir.
fn run_local(job: &str, workdir: &Path, cache_dir: &Path, env: WorkerEnv, gpu_lock: &Path) {
    let text = if job == "-" {
        std::io::read_to_string(std::io::stdin()).unwrap_or_default()
    } else {
        std::fs::read_to_string(job).unwrap_or_else(|e| {
            eprintln!("worker: {job}: {e}");
            std::process::exit(2);
        })
    };
    let spec: Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("worker: {job}: {e}");
        std::process::exit(2);
    });
    let kind = spec["kind"].as_str().unwrap_or_default();
    let all = handlers();
    let Some(handler) = all.iter().find(|h| h.kind() == kind) else {
        eprintln!("worker: unknown kind {kind:?} (see --kinds)");
        std::process::exit(2);
    };
    let ctx = JobContext {
        id: 0,
        dir: workdir.join(format!("local-{}", crate::unix_now())),
        repo: repo_root(),
        cache_dir: cache_dir.to_path_buf(),
        env,
    };
    let payload = spec
        .get("payload")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    match run_job(handler.as_ref(), &ctx, &payload, gpu_lock) {
        Ok(output) => {
            let path = ctx.dir.join(result_name(&output.content_type));
            if let Err(e) = std::fs::write(&path, &output.bytes) {
                eprintln!("worker: write {}: {e}", path.display());
                std::process::exit(1);
            }
            println!("summary: {}", output.summary);
            println!("result: {} ({} bytes)", path.display(), output.bytes.len());
        }
        Err(e) => {
            eprintln!("worker: {kind} failed: {}", e.message);
            std::process::exit(1);
        }
    }
}

/// `result.json` for JSON results, `result.bin` otherwise.
pub fn result_name(content_type: &str) -> &'static str {
    if content_type == "application/json" {
        "result.json"
    } else {
        "result.bin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_file_parses_and_hides_values() {
        let env = WorkerEnv::parse(
            "# plant worker\nexport APOSTILLE_WORKER_TOKEN=\"abc\"\n\
             META_API_KEY='k'\nEMPTY=\n junk\n",
        );
        assert_eq!(env.get("APOSTILLE_WORKER_TOKEN").as_deref(), Some("abc"));
        assert_eq!(env.get("META_API_KEY").as_deref(), Some("k"));
        assert_eq!(env.get("EMPTY"), None);
        let shown = format!("{env:?}");
        assert!(shown.contains("META_API_KEY") && !shown.contains("'k'") && !shown.contains("abc"));
    }

    #[cfg(unix)]
    #[test]
    fn env_file_must_be_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("worker.env");
        std::fs::write(&path, "APOSTILLE_WORKER_TOKEN=x\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(WorkerEnv::load(&path).unwrap_err().contains("chmod 600"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(WorkerEnv::load(&path).is_ok());
        assert!(WorkerEnv::load(&dir.path().join("missing")).is_ok());
    }

    #[test]
    fn singleton_lock_refuses_a_second_holder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w/worker.lock");
        let first = try_lock(&path).unwrap();
        assert!(first.is_some());
        assert!(try_lock(&path).unwrap().is_none());
        drop(first);
        assert!(try_lock(&path).unwrap().is_some());
    }

    struct GpuProbe;

    impl JobHandler for GpuProbe {
        fn kind(&self) -> &'static str {
            "test.gpu"
        }
        fn gpu(&self) -> bool {
            true
        }
        fn run(&self, ctx: &JobContext, _payload: &Value) -> Result<JobOutput, JobError> {
            // While a GPU job runs, nobody else can take the lock.
            let held = try_lock(&ctx.dir.join("../gpu.lock")).unwrap().is_none();
            Ok(JobOutput::json(
                &serde_json::json!({"held": held}),
                Value::Null,
            ))
        }
    }

    #[test]
    fn gpu_kinds_run_under_the_gpu_lock() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = JobContext {
            id: 1,
            dir: dir.path().join("job"),
            repo: repo_root(),
            cache_dir: dir.path().to_path_buf(),
            env: WorkerEnv::default(),
        };
        let out = run_job(&GpuProbe, &ctx, &Value::Null, &dir.path().join("gpu.lock")).unwrap();
        let v: Value = serde_json::from_slice(&out.bytes).unwrap();
        assert_eq!(v["held"], true);
        // flock belongs to the open file description: a subprocess that
        // another test forks at this moment shares it until its exec
        // closes the inherited fd, so allow a brief window for release.
        let lock = dir.path().join("gpu.lock");
        let released = (0..40).any(|_| {
            let free = try_lock(&lock).unwrap().is_some();
            if !free {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            free
        });
        assert!(released, "released after");
    }

    #[test]
    fn registry_kinds_are_unique_and_match_the_server_list() {
        let kinds: Vec<&str> = handlers().iter().map(|h| h.kind()).collect();
        let mut sorted = kinds.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), kinds.len());
        // The server's KINDS table (crates/web/src/worker.rs) names the same set.
        let server = include_str!("../../../crates/web/src/worker.rs");
        for kind in kinds {
            assert!(
                server.contains(&format!("kind: \"{kind}\"")),
                "{kind} missing on the server"
            );
        }
    }
}
