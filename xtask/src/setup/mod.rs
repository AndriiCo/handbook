//! `cargo xtask setup`: rebuild every target from one encrypted file.
//!
//! ```text
//! cargo xtask setup [--file private/config.yaml] [--target a,b] [--sha <rev>]
//!     [--from <phase>] [--dry-run] [--operator-order "<text>"]
//! cargo xtask setup wizard [--file <path>] [--host <name>] [--local] [--no-open]
//! cargo xtask setup check  [--file <path>] [--target <name>]
//! cargo xtask setup print  [--file <path>] --target <name>
//! cargo xtask setup new    [--file <path>] [--host <name>] [--local]
//! cargo xtask setup wipe   --target docker
//! ```
//!
//! With no file, `setup` creates one from the template and opens the
//! wizard on localhost. `--host` names the production server the
//! template adds (none without it). Without sops and an age key (or
//! with `--local`), the file is the plain local-only
//! `.local/setup.yaml` instead (0600, the local target only).
//!
//! With a file, it runs the file's
//! `default_targets` (or `--target`) phase by phase: doctor, gate, dns,
//! build, container, provision, seal, deploy, acme, staff, verify.
//! A server target named production refuses to run without
//! `--operator-order`, which is recorded in `private/setup-log.jsonl`.
//!
//! [`file`] reads and writes the sops file, [`build`] builds hardened
//! binaries for each target's CPU, [`plan`] turns a target into steps,
//! [`run`] executes them, [`checks`] probes every provider key,
//! [`probe`] reads what an SSH box is, [`map`] lays out the system map,
//! and [`wizard`] serves the browser UI.

pub mod brand;
pub mod build;
pub mod checks;
pub mod file;
pub mod map;
pub mod plan;
pub mod probe;
pub mod rehearsal;
pub mod run;
pub mod stage;
pub mod wizard;

use std::path::{Path, PathBuf};

use file::{SetupFile, TargetKind};
use plan::{Facts, Phase};
use run::Event;

fn fail(message: &str) -> ! {
    eprintln!("setup: {message}");
    std::process::exit(1);
}

/// `--file`; else `private/config.yaml` at the repo root (from any
/// subdirectory) when it exists; else the plain `.local/setup.yaml`
/// when it exists, `--local` asks for it, or sops/age are not set up;
/// else a new `private/config.yaml`.
fn file_path(args: &[String]) -> PathBuf {
    if let Some(path) = crate::flag(args, "file") {
        return PathBuf::from(path);
    }
    let repo = crate::repo_root();
    let (sops_file, plain) = (repo.join(file::DEFAULT_PATH), repo.join(file::LOCAL_PATH));
    if sops_file.exists() {
        return sops_file;
    }
    if plain.exists() || args.iter().any(|a| a == "--local") {
        return plain;
    }
    match file::sops_missing() {
        None => sops_file,
        Some(why) => {
            println!(
                "setup: {why}; using the plain local-only file {} (0600, local target only). Staging and production need sops and age.",
                file::LOCAL_PATH
            );
            plain
        }
    }
}

/// The template a new file at `path` starts from: the local target in
/// a plain file, else every target (production on `--host`).
pub fn new_file(path: &Path, args: &[String]) -> SetupFile {
    if file::is_plain(path) {
        return SetupFile::local_template();
    }
    let host = crate::flag(args, "host");
    if let Some(host) = host.as_deref()
        && !file::valid_host(host)
    {
        fail(&format!("--host {host:?} is not a host name"));
    }
    SetupFile::template_for(host.as_deref())
}

/// How a file at `path` is stored, for the "created" lines.
pub fn storage_note(path: &Path) -> &'static str {
    if file::is_plain(path) {
        "plain YAML, 0600, local target only"
    } else {
        "sops + age; plaintext never on disk"
    }
}

/// Load the file, or `None` when it does not exist yet.
pub fn load_or_none(path: &Path) -> Option<SetupFile> {
    match file::load(path) {
        Ok(f) => Some(f),
        Err(file::LoadError::NotFound) => None,
        Err(e) => fail(&e.to_string()),
    }
}

fn load_valid(path: &Path) -> SetupFile {
    let f = load_or_none(path).unwrap_or_else(|| {
        fail(&format!(
            "{} does not exist: run `cargo xtask setup` to create it in the wizard",
            path.display()
        ))
    });
    if let Err(problems) = f.validate() {
        fail(&format!("{}:\n  {}", path.display(), problems.join("\n  ")));
    }
    f
}

pub fn run(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("wizard") => wizard::run(&args[1..], &file_path(args)),
        Some("check") => run_check(&args[1..]),
        Some("print") => run_print(&args[1..]),
        Some("new") => run_new(&args[1..]),
        Some("wipe") => run_wipe(&args[1..]),
        Some("--help" | "-h" | "help") => print_usage(),
        Some(other) if !other.starts_with("--") => {
            eprintln!("setup: unknown subcommand {other:?}");
            print_usage();
            std::process::exit(2);
        }
        _ => run_targets(args),
    }
}

fn print_usage() {
    println!(
        "usage: cargo xtask setup [--file private/config.yaml] [--target a,b] [--sha <rev>] [--from <phase>] [--dry-run] [--operator-order \"<text>\"]
       cargo xtask setup wizard|check|print|new|wipe [--file <path>] [--target <name>]

No file: creates it and opens the wizard. Phases: {}",
        plan::ALL_PHASES.map(Phase::as_str).join(", ")
    );
}

fn run_new(args: &[String]) {
    let path = file_path(args);
    if path.exists() {
        fail(&format!(
            "{} exists; edit it with the wizard (or `sops {}`)",
            path.display(),
            path.display()
        ));
    }
    file::save(&path, &new_file(&path, args)).unwrap_or_else(|e| fail(&e));
    println!("created {} ({})", path.display(), storage_note(&path));
}

fn run_print(args: &[String]) {
    let f = load_valid(&file_path(args));
    let name = crate::required(args, "target");
    let values = f.target_config(&name).unwrap_or_else(|e| fail(&e));
    for (key, value) in values {
        let shown = if file::is_secret(&key) {
            "(secret, set)".to_string()
        } else {
            value.to_string()
        };
        println!("{key} = {shown}");
    }
}

fn run_wipe(args: &[String]) {
    let name = crate::required(args, "target");
    let f = load_valid(&file_path(args));
    match f.targets.get(&name).map(|t| t.kind) {
        Some(TargetKind::Docker) => {
            rehearsal::wipe(&crate::repo_root()).unwrap_or_else(|e| fail(&e))
        }
        Some(_) => {
            fail("only the docker rehearsal is wiped by setup; servers are wiped by their owner")
        }
        None => fail(&format!("no target {name:?}")),
    }
}

fn run_check(args: &[String]) {
    let f = load_valid(&file_path(args));
    let names: Vec<String> = match crate::flag(args, "target") {
        Some(t) => vec![t],
        None => f.targets.keys().cloned().collect(),
    };
    let mut bad = 0;
    for name in names {
        println!("== {name}");
        for r in checks::run_target(&f, &name) {
            println!("  {:<5} {:<22} {}", r.status.as_str(), r.label, r.detail);
            bad += usize::from(r.status == checks::Status::Fail);
        }
    }
    if bad > 0 {
        std::process::exit(1);
    }
}

/// CPU of a target, for the plan. A dry run falls back when the target
/// cannot be reached (and says so).
fn detect_arch(f: &SetupFile, name: &str) -> Result<build::Arch, String> {
    let target = f
        .targets
        .get(name)
        .ok_or_else(|| format!("no target {name:?}"))?;
    match target.kind {
        TargetKind::Local => build::host_triple().map(|t| {
            if t.starts_with("aarch64") {
                build::Arch::Aarch64
            } else {
                build::Arch::X86_64
            }
        }),
        TargetKind::Docker => {
            let daemon = build::detect_docker()?;
            match target.platform.as_deref().and_then(build::Arch::parse) {
                // systemd cannot set up its unit sandboxes under user-mode
                // emulation (sshd, dbus and tmp.mount fail), so the
                // rehearsal runs on the daemon's own CPU only.
                Some(arch) if arch != daemon => Err(format!(
                    "this Docker host is {} and systemd does not boot under {} emulation; \
                     rehearse {} on a {} Docker host (set platform: auto here)",
                    daemon.docker_platform(),
                    arch.docker_platform(),
                    arch.triple(),
                    arch.docker_platform()
                )),
                _ => Ok(daemon),
            }
        }
        TargetKind::Plant | TargetKind::Server => {
            build::detect_ssh(target.ssh.as_deref().unwrap_or_default(), None)
        }
    }
    .map_err(|e| format!("{name}: {e}"))
}

fn append_log(repo: &Path, entry: &serde_json::Value) {
    use std::io::Write as _;
    let path = repo.join("private/setup-log.jsonl");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{entry}");
    }
}

/// Print run events on the terminal.
pub fn print_event(event: Event) {
    match event {
        Event::Plan {
            target,
            version,
            steps,
        } => {
            println!("setup: {target} at {version}: {} steps", steps.len());
        }
        Event::Start {
            index,
            phase,
            title,
        } => println!("== {:>2} [{phase}] {title}", index + 1),
        Event::Line { text, .. } => println!("   {text}"),
        Event::Done { ok, detail, .. } => {
            if !ok {
                println!("   FAILED: {detail}");
            }
        }
        Event::Finished { ok, detail, .. } => {
            println!("setup: {}{detail}", if ok { "" } else { "STOPPED: " });
        }
    }
}

/// Run one target end to end (also used by the wizard).
pub fn run_one(
    f: &SetupFile,
    name: &str,
    rev: &str,
    from: Option<Phase>,
    order: Option<&str>,
    emit: &(dyn Fn(Event) + Sync),
) -> Result<(), String> {
    let repo = crate::repo_root();
    let target = f
        .targets
        .get(name)
        .ok_or_else(|| format!("no target {name:?}"))?;
    if target.kind == TargetKind::Server && name == "production" && order.is_none_or(str::is_empty)
    {
        return Err("production runs only on the operator's order: pass --operator-order \"<who, when, what>\"".into());
    }
    let sha = stage::resolve_sha(&repo, rev)?;
    let facts = Facts {
        arch: detect_arch(f, name)?,
        sha: sha.clone(),
    };
    let steps = plan::plan(f, name, &facts)?;
    append_log(
        &repo,
        &serde_json::json!({"at": crate::unix_now(), "target": name, "sha": sha, "order": order, "event": "start"}),
    );
    let ctx = run::Ctx {
        repo: repo.clone(),
        file: f,
        target: name.to_string(),
        facts,
        emit,
    };
    let result = run::execute(&ctx, &steps, from);
    append_log(
        &repo,
        &serde_json::json!({"at": crate::unix_now(), "target": name, "sha": sha, "ok": result.is_ok(), "event": "end"}),
    );
    result
}

fn run_targets(args: &[String]) {
    let path = file_path(args);
    let Some(f) = load_or_none(&path) else {
        println!(
            "setup: {} does not exist; creating it and opening the wizard",
            path.display()
        );
        file::save(&path, &new_file(&path, args)).unwrap_or_else(|e| fail(&e));
        wizard::run(args, &path);
        return;
    };
    if let Err(problems) = f.validate() {
        fail(&format!("{}:\n  {}", path.display(), problems.join("\n  ")));
    }
    let targets: Vec<String> = match crate::flag(args, "target") {
        Some(list) => list.split(',').map(str::to_string).collect(),
        None if !f.default_targets.is_empty() => f.default_targets.clone(),
        None => vec!["local".to_string()],
    };
    let rev = crate::flag(args, "sha").unwrap_or_else(|| "HEAD".to_string());
    let from = crate::flag(args, "from")
        .map(|p| Phase::parse(&p).unwrap_or_else(|| fail(&format!("unknown phase {p}"))));
    let order = crate::flag(args, "operator-order");
    if args.iter().any(|a| a == "--dry-run") {
        let repo = crate::repo_root();
        let sha = stage::resolve_sha(&repo, &rev).unwrap_or_else(|e| fail(&e));
        for name in &targets {
            let arch = detect_arch(&f, name).unwrap_or_else(|e| {
                println!("setup: ({e}; the dry run assumes x86_64)");
                build::Arch::X86_64
            });
            let facts = Facts {
                arch,
                sha: sha.clone(),
            };
            let steps = plan::plan(&f, name, &facts).unwrap_or_else(|e| fail(&e));
            println!("# {name} at {} ({})", facts.version(), facts.arch.triple());
            print!("{}", plan::render(&steps));
        }
        return;
    }
    for name in &targets {
        if let Err(e) = run_one(&f, name, &rev, from, order.as_deref(), &print_event) {
            // The run already printed its own STOPPED line; only refusals
            // before the first step still need saying.
            if !e.starts_with("step ") {
                eprintln!("setup: {e}");
            }
            std::process::exit(1);
        }
    }
}
