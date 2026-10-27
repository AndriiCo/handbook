//! Project task runner. Use `cargo xtask <task>` instead of shell scripts.
//!
//! Keeping tasks in Rust reuses the project's language, libraries, and
//! error handling instead of scattering logic across shell dialects.

use std::process::Command;

use handbook::store::Store;

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
    handbook::db::open(store).unwrap_or_else(|e| {
        eprintln!("open database (run `cargo xtask init` first): {e}");
        std::process::exit(1);
    })
}

fn run_ci() {
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
    if let Err(e) = handbook::db::migrate(&conn) {
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
    let new = handbook::db::NewAuthority {
        kind: &kind,
        country: &country,
        region: &region,
        name: &name,
        slug: &slug,
        actor: &actor,
        at: unix_now(),
    };
    match handbook::db::add_authority(&conn, &new) {
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
    match handbook::db::retention_sweep(&conn, &store, now, &actor) {
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
