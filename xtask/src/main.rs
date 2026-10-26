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

add-authority flags:
  --store <path>  store root (default: store)
  --kind <kind>   country | state | province | office (required)
  --country <c>   e.g. USA (required)
  --region <r>    e.g. Washington (default: empty)
  --name <n>      display name (required)
  --slug <s>      unique slug, e.g. usa-wa-sos (required)"
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
    match handbook::db::add_authority(
        &conn,
        &required(args, "kind"),
        &required(args, "country"),
        &region,
        &required(args, "name"),
        &required(args, "slug"),
    ) {
        Ok(id) => println!("authority id: {id}"),
        Err(e) => {
            eprintln!("add-authority: {e}");
            std::process::exit(1);
        }
    }
}

fn run_retention_sweep(args: &[String]) {
    let store = open_store(args);
    let conn = open_db(&store);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before 1970")
        .as_secs()
        .try_into()
        .expect("unix time exceeds i64");
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    match handbook::db::retention_sweep(&conn, &store, now, &actor) {
        Ok(deleted) => println!("expired audio captures deleted: {deleted}"),
        Err(e) => {
            eprintln!("retention-sweep: {e}");
            std::process::exit(1);
        }
    }
}
