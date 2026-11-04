//! `cargo xtask worker` — plant worker for the production work queue
//! (P392, spec `docs/specs/work-queue.md`; enrolment and settings:
//! `docs/specs/worker-enrolment.md`).
//!
//! ```text
//! cargo xtask worker --connect wss://andrii.ai/apostille/api/v1/worker/ws
//!     [--token-file ~/.config/apostille/worker.token]
//!     [--workdir /srv/scratch/apostille/work] [--cache-dir /srv/scratch/apostille]
//!     [--gpu-lock ~/apostille-pipeline/gpu.lock] [--once]
//! cargo xtask worker --run-local <job.json> [--settings <settings.json>] [--workdir …]
//! cargo xtask worker --unpack <bundle.json> [--into packs/world-nonparty]
//! cargo xtask worker --kinds [--settings <settings.json>]
//! ```
//!
//! Dials OUT to production, runs one job at a time, reconnects with
//! backoff, and holds `<workdir>/worker.lock` so a cron watchdog can
//! start it every minute (the second copy exits quietly). GPU kinds
//! also hold `~/apostille-pipeline/gpu.lock` while they run.
//!
//! The credential lives in the token file (see [`enrol`] for its
//! format). Without one the worker enrols: it logs a pairing code that
//! staff approve on Work queue → Workers, then saves the token it is
//! given. Job settings (the Meta key, model ids, modes) arrive in
//! `welcome.settings` and are held in memory only: the worker reads no
//! `.env` file and no setting from its own environment.
//!
//! New kinds are one file each: implement [`JobHandler`] and add the
//! handler to [`handlers`] (and the server's `KINDS` list in
//! `crates/web/src/worker.rs`).

mod client;
mod discovery_bootstrap;
pub(crate) mod enrol;
mod eval_gold;
mod precheck_glimmer;
pub(crate) mod research_nonparty;
mod scrape_urls;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::flag;

/// Default scratch layout on plant (operator decision 2026-11-03).
const DEFAULT_WORKDIR: &str = "/srv/scratch/apostille/work";
/// The worker's credential (0600 in a 0700 directory, never in the repo).
const DEFAULT_TOKEN_FILE: &str = ".config/apostille/worker.token";
/// Shared with the plant build pipeline's GPU stages.
const DEFAULT_GPU_LOCK: &str = "apostille-pipeline/gpu.lock";

/// One job's environment: where to write, where the repo is, settings.
pub struct JobContext {
    pub id: i64,
    /// Per-attempt scratch dir (`<workdir>/job-<id>-a<attempt>`).
    pub dir: PathBuf,
    /// Repository checkout (packs, fixtures, gold sets).
    pub repo: PathBuf,
    /// Cross-job caches (`<cache-dir>/fetch-cache`, …).
    pub cache_dir: PathBuf,
    /// `welcome.settings` as of the job's start (memory only).
    pub settings: Settings,
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
    /// stay pending until a worker that can run them connects.
    /// `settings` are the last `welcome.settings` (empty before the
    /// first `welcome`; the client says `hello` again when they change
    /// what it can announce).
    fn available(&self, _settings: &Settings) -> bool {
        true
    }
    fn run(&self, ctx: &JobContext, payload: &Value) -> Result<JobOutput, JobError>;
}

/// Every handler this worker knows.
pub fn handlers() -> Vec<Box<dyn JobHandler>> {
    vec![
        Box::new(research_nonparty::ResearchNonparty),
        Box::new(discovery_bootstrap::DiscoveryBootstrap),
        Box::new(scrape_urls::ScrapeUrls),
        Box::new(eval_gold::EvalGold),
        Box::new(precheck_glimmer::PrecheckGlimmer),
    ]
}

/// `welcome.settings` keys (`docs/specs/worker-enrolment.md` §6).
pub mod keys {
    /// Production's `meta.api_key` (never logged, never on disk).
    pub const META_API_KEY: &str = "meta_api_key";
    pub const META_BASE_URL: &str = "meta_base_url";
    pub const META_CACHE_MODE: &str = "meta_cache_mode";
    pub const META_DAILY_CALL_CAP: &str = "meta_daily_call_cap";
    /// The fetch cache's mode (was `FETCH_CACHE_MODE`; the server sends
    /// it only if it has a value, else the default mode applies).
    pub const FETCH_CACHE_MODE: &str = "fetch_cache_mode";
    /// The server's effective `hf.mode` (`fake` on `cargo xtask up`).
    pub const HF_MODE: &str = "hf_mode";
    pub const GLIMMER_MODEL: &str = "glimmer_model";
    pub const GLIMMER_URL: &str = "glimmer_url";
    /// `eval.gold --local` endpoint fallbacks (the payload's `base_url`
    /// and `model` come first; the server sends these only if a row
    /// for them exists).
    pub const LOCAL_MODEL_URL: &str = "local_model_url";
    pub const LOCAL_MODEL: &str = "local_model";
    pub const LOCAL_MODEL_KEY: &str = "local_model_key";
}

/// The server's `welcome.settings`: string → string, absent = unset.
/// Held in memory only: never written to disk, never put in the process
/// environment, and Debug prints the keys, never the values.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Settings {
    values: BTreeMap<String, String>,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys: Vec<&String> = self.values.keys().collect();
        f.debug_struct("Settings").field("keys", &keys).finish()
    }
}

impl Settings {
    /// From a JSON object of strings (`welcome.settings`, or a
    /// `--settings` file). A missing/null value is empty settings; a
    /// non-string value is refused.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        match value {
            Value::Null => {}
            Value::Object(map) => {
                for (key, v) in map {
                    let Some(s) = v.as_str() else {
                        return Err(format!("settings.{key} must be a string"));
                    };
                    values.insert(key.clone(), s.to_string());
                }
            }
            _ => return Err("settings must be a JSON object of strings".to_string()),
        }
        Ok(Self { values })
    }

    /// The value for `key` (an empty string is unset).
    pub fn get(&self, key: &str) -> Option<String> {
        self.values.get(key).filter(|v| !v.is_empty()).cloned()
    }

    /// `--settings <file>` for `--run-local` / `--kinds`: the same keys
    /// as `welcome.settings`. It may hold the Meta key, so a file other
    /// users can read is refused (0600 rule).
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        check_private(path)?;
        let value: Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_json(&value).map_err(|e| format!("{}: {e}", path.display()))
    }

    #[cfg(test)]
    pub fn of(pairs: &[(&str, &str)]) -> Self {
        Self {
            values: pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        }
    }
}

/// Refuse a secret file whose mode lets group or other read it.
pub(crate) fn check_private(path: &Path) -> Result<(), String> {
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
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
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

/// `--settings <file>` when given (exits on a bad file), else empty.
fn settings_flag(args: &[String]) -> Settings {
    match flag(args, "settings") {
        Some(path) => Settings::load(Path::new(&path)).unwrap_or_else(|e| {
            eprintln!("worker: {e}");
            std::process::exit(2);
        }),
        None => Settings::default(),
    }
}

pub fn run(args: &[String]) {
    let started_at = crate::unix_now();
    if args.iter().any(|a| a == "--kinds") {
        let settings = settings_flag(args);
        for h in handlers() {
            let gpu = if h.gpu() { " (gpu)" } else { "" };
            let ready = if h.available(&settings) {
                ""
            } else {
                " [not announced with these settings]"
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
    if let Some(job) = flag(args, "run-local") {
        let settings = settings_flag(args);
        return run_local(&job, &workdir, &cache_dir, settings, &gpu_lock);
    }
    let Some(url) = flag(args, "connect") else {
        eprintln!("worker: --connect <wss url>, --run-local <job.json>, --unpack or --kinds");
        std::process::exit(2);
    };
    let token_path =
        flag(args, "token-file").map_or_else(|| home().join(DEFAULT_TOKEN_FILE), PathBuf::from);
    // Fail on a bad URL before taking the lock or enrolling.
    let enrol_url = enrol::enrol_url(&url).unwrap_or_else(|e| {
        eprintln!("worker: {e}");
        std::process::exit(2);
    });
    // Held for the whole run, enrolment included: one open request and
    // one connection per workdir.
    let _singleton = match try_lock(&workdir.join("worker.lock")) {
        Ok(Some(file)) => file,
        Ok(None) => {
            // Cron starts one every minute: say so only to a person.
            if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
                eprintln!(
                    "worker: already running ({} held)",
                    workdir.join("worker.lock").display()
                );
            }
            return;
        }
        Err(e) => {
            eprintln!("worker: {e}");
            std::process::exit(1);
        }
    };
    // A drain left from before this start is stale: this is the new run.
    let drain = workdir.join("worker.drain");
    let _ = std::fs::remove_file(&drain);
    let repo = repo_root();
    let all = std::sync::Arc::new(handlers());
    let mut cfg = client::Config {
        url,
        token: String::new(),
        name: String::new(),
        workdir,
        cache_dir,
        version: version(&repo),
        repo,
        gpu_lock,
        once: args.iter().any(|a| a == "--once"),
        drain,
    };
    loop {
        let credential = match enrol::load_credential(&token_path) {
            Ok(Some(credential)) => credential,
            Ok(None) => {
                let request = enrol::Request {
                    url: enrol_url.clone(),
                    token_path: token_path.clone(),
                    name: enrol::proposed_name(),
                    version: cfg.version.clone(),
                    started_at,
                    capabilities: all.iter().map(|h| h.kind().to_string()).collect(),
                };
                match enrol::enrol(&request, &enrol::UreqHttp::new()) {
                    Ok(credential) => credential,
                    Err(why) => {
                        // Cron starts the worker again next minute, and
                        // that run opens a fresh request.
                        eprintln!("worker: {why}");
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("worker: {e}");
                std::process::exit(2);
            }
        };
        cfg.token = credential.token;
        cfg.name = credential.worker;
        match client::run_with(&cfg, std::sync::Arc::clone(&all)) {
            client::Exit::Done => return,
            client::Exit::Unauthorized(why) => match enrol::retire_credential(&token_path) {
                Ok(to) => eprintln!(
                    "worker: the server refused this credential ({why}); moved it to {} \
                         and enrolling again",
                    to.display()
                ),
                Err(e) => {
                    eprintln!("worker: the server refused this credential ({why}); {e}");
                    std::process::exit(1);
                }
            },
        }
    }
}

/// `<semver>+<sha>` for `hello`: the `.sha` file `worker-install.sh`
/// writes into the source tree (plant builds from `git archive`, no
/// `.git`), else `git rev-parse` in a checkout, else the semver alone.
fn version(repo: &Path) -> String {
    let from_file = std::fs::read_to_string(repo.join(".sha")).ok();
    let from_git = || {
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let sha = from_file
        .or_else(from_git)
        .map(|s| s.trim().chars().take(12).collect::<String>())
        .filter(|s| s.len() >= 7 && s.bytes().all(|b| b.is_ascii_hexdigit()));
    match sha {
        Some(sha) => format!("{}+{sha}", env!("CARGO_PKG_VERSION")),
        None => env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// `--run-local <job.json>`: `{kind, payload}` from a file (or `-`
/// for stdin), run here with `--settings` (no server, so no
/// `welcome`), result written next to the job dir.
fn run_local(job: &str, workdir: &Path, cache_dir: &Path, settings: Settings, gpu_lock: &Path) {
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
        settings,
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
    fn settings_parse_strings_and_hide_values() {
        let settings = Settings::from_json(&serde_json::json!({
            "meta_api_key": "LLM_secret", "hf_mode": "fake", "glimmer_url": ""
        }))
        .unwrap();
        assert_eq!(
            settings.get(keys::META_API_KEY).as_deref(),
            Some("LLM_secret")
        );
        assert_eq!(settings.get(keys::HF_MODE).as_deref(), Some("fake"));
        assert_eq!(settings.get(keys::GLIMMER_URL), None, "empty is unset");
        assert_eq!(settings.get(keys::GLIMMER_MODEL), None, "absent is unset");
        let shown = format!("{settings:?}");
        assert!(shown.contains("meta_api_key") && !shown.contains("LLM_secret"));
        assert!(Settings::from_json(&serde_json::json!({"x": 1})).is_err());
        assert!(Settings::from_json(&serde_json::json!(["x"])).is_err());
        assert_eq!(
            Settings::from_json(&Value::Null).unwrap(),
            Settings::default()
        );
    }

    #[cfg(unix)]
    #[test]
    fn settings_file_must_be_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"meta_api_key": "k", "hf_mode": "fake"}"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Settings::load(&path).unwrap_err().contains("chmod 600"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().get(keys::HF_MODE).as_deref(),
            Some("fake")
        );
        assert!(Settings::load(&dir.path().join("missing")).is_err());
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
            settings: Settings::default(),
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
