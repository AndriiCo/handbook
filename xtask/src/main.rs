//! Project task runner. Use `cargo xtask <task>` instead of shell scripts.
//!
//! Keeping tasks in Rust reuses the project's language, libraries, and
//! error handling instead of scattering logic across shell dialects.

use std::process::Command;

use handbook_store::store::Store;

mod secrets;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("--help" | "-h" | "help") => print_help(),
        Some("ci") => run_ci(),
        Some("init") => run_init(&args[1..]),
        Some("add-authority") => run_add_authority(&args[1..]),
        Some("retention-sweep") => run_retention_sweep(&args[1..]),
        Some("web-budget") => run_web_budget(),
        Some("serve") => run_serve(),
        Some("analyze") => run_analyze(&args[1..]),
        Some("secrets-scan") => run_secrets_scan(&args[1..]),
        Some("install-hooks") => run_install_hooks(),
        Some("audit") => run_audit(),
        Some(unknown) => {
            eprintln!("unknown task: {unknown}\n");
            print_help();
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!(
        "Usage: cargo xtask <task> [flags]

Tasks:
  ci     fmt --check, clippy, and tests (what CI runs)
  init   create a store and apply migrations [--store <path>]
  add-authority   add an office/jurisdiction (see flags below)
  retention-sweep delete voice audio older than 365 days [--store <path>] [--actor <name>]
  web-budget  check web/build JS+CSS against the Day-1 150KB gate
  serve       run the dev API server (cargo run -p web)
  analyze     draft proposed facts for an authority (stub only for now)
  secrets-scan scan tracked files for secrets [--staged for the index]
  install-hooks point git at .githooks/ (pre-commit secret scan)
  audit       cargo-audit over Cargo.lock (needs network; not in ci)

analyze flags:
  --store <path>  store root (default: store)
  --slug <s>      authority slug, e.g. usa-wa-sos (required)
  --input <text>  evidence text, or @path/to/file (required)
  --actor <a>     audit actor (default: xtask)

add-authority flags:
  --store <path>  store root (default: store)
  --kind <kind>   country | state | province | office (required)
  --country <c>   e.g. USA (required)
  --region <r>    e.g. Washington (default: empty)
  --name <n>      display name (required)
  --slug <s>      unique slug, e.g. usa-wa-sos (required)
  --actor <a>     audit actor (default: xtask)"
    );
}

/// Value of `--name <value>` in task args, if present.
fn flag(args: &[String], name: &str) -> Option<String> {
    args.windows(2).find_map(|pair| {
        if pair[0] == format!("--{name}") {
            Some(pair[1].clone())
        } else {
            None
        }
    })
}

fn required(args: &[String], name: &str) -> String {
    flag(args, name).unwrap_or_else(|| {
        eprintln!("missing required flag: --{name}");
        std::process::exit(2);
    })
}

fn open_store(args: &[String]) -> Store {
    Store::new(flag(args, "store").unwrap_or_else(|| "store".to_string()))
}

/// Database handle for an already-opened store (F6: one open path — tasks
/// must not reimplement the open-and-handle-error sequence themselves).
fn open_db(store: &Store) -> rusqlite::Connection {
    handbook_store::db::open(store).unwrap_or_else(|e| {
        eprintln!("open database (run `cargo xtask init` first): {e}");
        std::process::exit(1);
    })
}

fn run_ci() {
    // Secrets first: fail fast before compiling anything else.
    run_secrets_scan(&[]);
    for args in [
        ["fmt", "--all", "--check"].as_slice(),
        [
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ]
        .as_slice(),
        ["test", "--workspace"].as_slice(),
    ] {
        let status = Command::new("cargo")
            .args(args)
            .status()
            .expect("failed to run cargo");
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
    }
}

fn run_init(args: &[String]) {
    let store = open_store(args);
    if let Err(e) = store.init() {
        eprintln!("init store dirs: {e}");
        std::process::exit(1);
    }
    let conn = open_db(&store);
    if let Err(e) = handbook_store::db::migrate(&conn) {
        eprintln!("migrate: {e}");
        std::process::exit(1);
    }
    println!("store ready: {}", store.root().display());
}

fn run_add_authority(args: &[String]) {
    let store = open_store(args);
    let conn = open_db(&store);
    let region = flag(args, "region").unwrap_or_default();
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let kind = required(args, "kind");
    let country = required(args, "country");
    let name = required(args, "name");
    let slug = required(args, "slug");
    let new = handbook_store::db::NewAuthority {
        kind: &kind,
        country: &country,
        region: &region,
        name: &name,
        slug: &slug,
        actor: &actor,
        at: unix_now(),
    };
    match handbook_store::db::add_authority(&conn, &new) {
        Ok(id) => println!("authority id: {id}"),
        Err(e) => {
            eprintln!("add-authority: {e}");
            std::process::exit(1);
        }
    }
}

/// Current unix time as i64 (audit `at` values).
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before 1970")
        .as_secs()
        .try_into()
        .expect("unix time exceeds i64")
}

fn run_retention_sweep(args: &[String]) {
    let store = open_store(args);
    let conn = open_db(&store);
    let now = unix_now();
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    match handbook_store::db::retention_sweep(&conn, &store, now, &actor) {
        Ok(deleted) => println!("expired audio captures deleted: {deleted}"),
        Err(e) => {
            eprintln!("retention-sweep: {e}");
            std::process::exit(1);
        }
    }
}

/// Day-1 JS budget gate: raw JS+CSS bytes in `web/build` must stay
/// under 150KB (cold cache, excluding images/fonts).
fn run_web_budget() {
    const BUDGET: u64 = 150 * 1024;
    let mut sized: Vec<(u64, String)> = Vec::new();
    let mut stack = vec![std::path::PathBuf::from("web/build")];
    if !stack[0].is_dir() {
        eprintln!("web-budget: web/build missing (run `npm run build` in web/ first)");
        std::process::exit(1);
    }
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| {
            eprintln!("web-budget: read {}: {e}", dir.display());
            std::process::exit(1);
        });
        for entry in entries {
            let entry = entry.unwrap_or_else(|e| {
                eprintln!("web-budget: entry: {e}");
                std::process::exit(1);
            });
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("js" | "css")
            ) {
                let bytes = entry
                    .metadata()
                    .unwrap_or_else(|e| {
                        eprintln!("web-budget: stat {}: {e}", path.display());
                        std::process::exit(1);
                    })
                    .len();
                sized.push((bytes, path.display().to_string()));
            }
        }
    }
    sized.sort();
    sized.reverse();
    let total: u64 = sized.iter().map(|(b, _)| b).sum();
    for (bytes, path) in sized.iter().take(8) {
        println!("{bytes:>8}  {path}");
    }
    println!("total JS+CSS: {total} bytes (budget: {BUDGET})");
    if total > BUDGET {
        eprintln!("web-budget: OVER BUDGET by {} bytes", total - BUDGET);
        std::process::exit(1);
    }
}

fn run_serve() {
    let status = Command::new("cargo")
        .args(["run", "-p", "web"])
        .status()
        .expect("failed to run cargo");
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn run_analyze(args: &[String]) {
    use handbook_agent::model::StubClient;
    use std::collections::HashMap;

    let store = open_store(args);
    let conn = open_db(&store);
    if let Err(e) = handbook_store::db::migrate(&conn) {
        eprintln!("migrate: {e}");
        std::process::exit(1);
    }
    let slug = required(args, "slug");
    let raw_input = required(args, "input");
    let input = match raw_input.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("read input file: {e}");
            std::process::exit(1);
        }),
        None => raw_input,
    };
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    // Stub-only for now: the stub cites doc_id "fixture", so the CLI
    // evidence IS the fixture text. Live Meta calls arrive with the
    // MetaRestClient HTTP wiring (roadmap S2 Day 3).
    let client = StubClient;
    let mut sources = HashMap::new();
    sources.insert("fixture".to_string(), input.clone());
    match handbook_agent::runner::run_analysis(
        &conn,
        &client,
        &slug,
        &input,
        &sources,
        &actor,
        unix_now(),
    ) {
        Ok(report) => {
            println!(
                "authority: {}\nproposed: {}\nrejected: {}",
                report.authority_slug, report.proposed, report.rejected
            );
            for error in &report.errors {
                println!("rejected: {error}");
            }
        }
        Err(e) => {
            eprintln!("analyze: {e}");
            std::process::exit(1);
        }
    }
}

/// NUL-separated paths from a git listing command.
fn git_paths(args: &[&str]) -> Vec<String> {
    let output = Command::new("git").args(args).output().unwrap_or_else(|e| {
        eprintln!("secrets-scan: git failed: {e}");
        std::process::exit(1);
    });
    if !output.status.success() {
        eprintln!("secrets-scan: git failed (not a repo?)");
        std::process::exit(1);
    }
    output
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect()
}

/// Scan tracked files, or `--staged` the index content (pre-commit).
/// Prints `path:line: rule` hits; exits 1 on any hit (fail closed).
fn run_secrets_scan(args: &[String]) {
    let staged = args.iter().any(|a| a == "--staged");
    let paths = if staged {
        git_paths(&[
            "diff",
            "--cached",
            "--name-only",
            "-z",
            "--no-renames",
            "--diff-filter=ACMR",
        ])
    } else {
        git_paths(&["ls-files", "-z"])
    };
    let mut hits = 0;
    for path in &paths {
        if path.contains("target/")
            || path.contains("node_modules/")
            || path.contains(".svelte-kit/")
        {
            continue;
        }
        let bytes = if staged {
            let spec = format!(":{path}");
            match Command::new("git").args(["show", &spec]).output() {
                Ok(o) if o.status.success() => o.stdout,
                _ => {
                    eprintln!("secrets-scan: cannot read staged {path}");
                    std::process::exit(1);
                }
            }
        } else {
            match std::fs::read(path) {
                Ok(b) => b,
                Err(_) => continue, // deleted after listing; ignore
            }
        };
        for hit in secrets::scan_bytes(path, &bytes) {
            println!("{}:{}: {}", hit.file, hit.line, hit.rule);
            hits += 1;
        }
    }
    if hits > 0 {
        eprintln!("secrets-scan: {hits} finding(s) — refusing");
        std::process::exit(1);
    }
    eprintln!(
        "secrets-scan: clean ({} file(s){})",
        paths.len(),
        if staged { ", staged" } else { "" }
    );
}

/// Point this clone's git at the tracked `.githooks/` dir.
fn run_install_hooks() {
    let status = Command::new("git")
        .args(["config", "core.hooksPath", ".githooks"])
        .status()
        .unwrap_or_else(|e| {
            eprintln!("install-hooks: git failed: {e}");
            std::process::exit(1);
        });
    if !status.success() {
        eprintln!("install-hooks: git config failed");
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let hook = std::path::Path::new(".githooks/pre-commit");
        let mut perms = std::fs::metadata(hook)
            .unwrap_or_else(|e| {
                eprintln!("install-hooks: read {}: {e}", hook.display());
                std::process::exit(1);
            })
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(hook, perms).unwrap_or_else(|e| {
            eprintln!("install-hooks: chmod {}: {e}", hook.display());
            std::process::exit(1);
        });
    }
    println!("hooks installed: core.hooksPath=.githooks");
}

/// Surface `cargo audit` as a task. Deliberately NOT part of `ci`
/// (needs the network + the plugin); run before releases.
fn run_audit() {
    let status = Command::new("cargo").args(["audit"]).status();
    match status {
        Ok(s) if s.success() => println!("audit: clean"),
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("audit: cargo-audit not installed ({e}); run `cargo install cargo-audit`");
            std::process::exit(1);
        }
    }
}
