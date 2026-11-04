//! `cargo xtask judge`: the one command a hackathon judge runs on a
//! fresh clone (macOS or Linux, Rust + Node) to get a working local
//! copy — public site on http://localhost:18318/apostille, admin on
//! http://localhost:18319 (`--port-base N` moves both) — with no keys, synthetic data and recorded
//! Meta model responses (`--live` drafts through the judge's own
//! `META_API_KEY` instead).
//!
//! Flow: a prerequisite doctor (one exact fix per failure, nothing
//! installed for you), then a fresh `up` boot labelled `judge`, then a
//! short tour. `--doctor` runs the checks only; `--check` boots,
//! probes, runs one job through the local worker and exits (the plant
//! gate runs it after `cargo xtask ci`: `ops/plant-pipeline/worker.sh`).
//! The doctor needs xtask compiled first; `ops/judge/preflight.sh`
//! checks the C compiler, rustup, Node and npm before cargo builds.

use std::net::TcpListener;
use std::process::Command;

/// Free disk the first build needs (release target + node_modules +
/// both SPAs + the store), with headroom.
const MIN_FREE_GB: u64 = 6;
/// `web/package.json` engines.
const NODE_MIN_MAJOR: u64 = 24;
const NPM_MIN_MAJOR: u64 = 11;
/// npm 10 still boots: `up` installs with `--legacy-peer-deps`.
const NPM_FALLBACK_MAJOR: u64 = 10;

/// One doctor finding: what failed and the exact command that fixes it.
struct Problem {
    what: String,
    fix: String,
    /// A warning prints but does not stop the boot.
    warning: bool,
}

impl Problem {
    fn new(what: String, fix: impl Into<String>) -> Self {
        Self {
            what,
            fix: fix.into(),
            warning: false,
        }
    }
}

/// Entry point (the builder wires the `judge` arm in `main.rs`).
pub fn run(args: &[String]) {
    let doctor_only = args.iter().any(|a| a == "--doctor");
    if let Err(e) = crate::up::apply_port_base(args) {
        eprintln!("judge: {e}");
        std::process::exit(2);
    }
    println!("judge: checking prerequisites (nothing is installed for you)");
    let (warnings, problems): (Vec<Problem>, Vec<Problem>) =
        doctor(args).into_iter().partition(|p| p.warning);
    for w in &warnings {
        println!("  ! {}", w.what);
        println!("    fix (optional): {}", w.fix);
    }
    if !problems.is_empty() {
        eprintln!();
        eprintln!("judge: {} prerequisite(s) to fix first:", problems.len());
        for p in &problems {
            eprintln!("  ✗ {}", p.what);
            eprintln!("    fix: {}", p.fix);
        }
        eprintln!();
        if prebuilt(args) {
            eprintln!("Then start it again.");
        } else {
            eprintln!("Then run `cargo xtask judge` again.");
        }
        std::process::exit(1);
    }
    println!("judge: all prerequisites ok");
    if doctor_only {
        return;
    }
    if !prebuilt(args) {
        println!(
            "judge: first run builds both web apps and the release binaries — about 8-12 minutes; reruns take a minute or two"
        );
    }
    crate::up::set_env_label("judge");
    let mut forwarded: Vec<String> = vec!["--fresh".to_string()];
    forwarded.extend(args.iter().filter(|a| *a != "--doctor").cloned());
    crate::up::run(&forwarded);
}

/// `--prebuilt` (the Docker image): binaries and both SPA builds are
/// already there, so only the key and port checks apply.
fn prebuilt(args: &[String]) -> bool {
    args.iter().any(|a| a == "--prebuilt")
}

fn doctor(args: &[String]) -> Vec<Problem> {
    let mut problems = Vec::new();
    if args.iter().any(|a| a == "--live")
        && !std::env::var("META_API_KEY").is_ok_and(|k| !k.trim().is_empty())
    {
        problems.push(Problem::new(
            "--live drafts through the Meta Model API, and META_API_KEY is not set".to_string(),
            "set META_API_KEY in the environment (or drop --live for the recorded responses)",
        ));
    }
    if prebuilt(args) {
        problems.extend(port_problems());
        return problems;
    }
    match std::env::consts::OS {
        "macos" | "linux" => {}
        other => problems.push(Problem::new(
            format!("unsupported OS {other:?} (macOS or Linux only)"),
            "run it on macOS or Linux (WSL2 works on Windows)",
        )),
    }
    if version_of("git", &["--version"]).is_none() {
        problems.push(Problem::new(
            "git is not on PATH".to_string(),
            "macOS: xcode-select --install · Debian/Ubuntu: sudo apt install git",
        ));
    }
    match version_of("node", &["--version"]) {
        None => problems.push(Problem::new(
            "Node.js is not on PATH".to_string(),
            format!("install Node {NODE_MIN_MAJOR}+: nvm install {NODE_MIN_MAJOR} && nvm use {NODE_MIN_MAJOR}"),
        )),
        Some(v) if major(&v).is_some_and(|m| m < NODE_MIN_MAJOR) => problems.push(Problem::new(
            format!("Node {v} is older than {NODE_MIN_MAJOR}"),
            format!("nvm install {NODE_MIN_MAJOR} && nvm use {NODE_MIN_MAJOR}"),
        )),
        Some(_) => {}
    }
    match version_of("npm", &["--version"]) {
        None => problems.push(Problem::new(
            "npm is not on PATH".to_string(),
            format!("it ships with Node {NODE_MIN_MAJOR}+: nvm install {NODE_MIN_MAJOR}"),
        )),
        Some(v) => {
            if let Some(p) = npm_finding(&v) {
                problems.push(p);
            }
        }
    }
    problems.extend(port_problems());
    if let Some(free) = free_gb(".")
        && free < MIN_FREE_GB
    {
        problems.push(Problem::new(
            format!("only {free} GB free here; the first build needs about {MIN_FREE_GB} GB"),
            "free disk space, or clone into a roomier volume",
        ));
    }
    if std::path::Path::new("Cargo.toml").exists()
        && !std::path::Path::new("web/package.json").exists()
    {
        problems.push(Problem::new(
            "not at the repository root".to_string(),
            "cd into the cloned repository, then cargo xtask judge",
        ));
    }
    problems
}

/// The public and admin ports must be free on 127.0.0.1.
fn port_problems() -> Vec<Problem> {
    [crate::up::public_port(), crate::up::admin_port()]
        .into_iter()
        .filter(|port| TcpListener::bind(("127.0.0.1", *port)).is_err())
        .map(|port| {
            Problem::new(
                format!("port {port} is in use on 127.0.0.1"),
                format!(
                    "stop whatever holds it (lsof -nP -iTCP:{port} -sTCP:LISTEN), or pick other ports: --port-base 28318"
                ),
            )
        })
        .collect()
}

/// npm below the engines floor: 10 is a warning (`up` installs with
/// `--legacy-peer-deps`), anything older stops the boot.
fn npm_finding(version: &str) -> Option<Problem> {
    let m = major(version)?;
    if m >= NPM_MIN_MAJOR {
        return None;
    }
    let mut p = Problem::new(
        format!("npm {version} is older than {NPM_MIN_MAJOR} (web/package.json engines)"),
        format!("npm install -g npm@{NPM_MIN_MAJOR}"),
    );
    if m >= NPM_FALLBACK_MAJOR {
        p.what
            .push_str("; the boot continues with `npm ci --legacy-peer-deps`");
        p.warning = true;
    }
    Some(p)
}

/// First line of `cmd args` stdout, trimmed (`None` when absent).
fn version_of(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status.success().then(|| {
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string()
    })
}

/// Major version from `v24.1.0` / `11.2.0` / `git version 2.4`.
fn major(version: &str) -> Option<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())?
        .parse()
        .ok()
}

/// Free GB on the filesystem holding `path` (`df -Pk`, POSIX output).
fn free_gb(path: &str) -> Option<u64> {
    let out = Command::new("df").args(["-Pk", path]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let kb: u64 = text
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()?;
    Some(kb / 1024 / 1024)
}

/// The 5-minute tour printed under `up`'s banner (`docs/judges.md`).
pub fn print_tour(public_port: u16, admin_port: u16, setup_open: bool, prebuilt: bool) {
    println!();
    println!("judge tour (5 minutes; details in docs/judges.md):");
    println!("  1. http://localhost:{public_port}/apostille — press ⌘K, search \"Washington\"");
    if setup_open {
        println!(
            "  2. http://localhost:{admin_port}/config first — paste the setup key above, enrol a passkey"
        );
    } else {
        println!("  2. http://localhost:{admin_port}/staff-login — sign in with your passkey");
    }
    println!(
        "  3. Review: each Muse Spark draft shows its quote; accept a few with the dossier open in"
    );
    println!("     another tab — it updates live over WebSocket, no reload");
    println!(
        "  4. the guide: citations are off by default; press Citations or Alt+C (⌥C on a Mac);"
    );
    println!("     hover, focus or tap a marker for its quote, source and capture date");
    println!(
        "  5. Trace: \"Draft from this capture\" streams a model run (recorded unless --live)"
    );
    if prebuilt {
        println!("  stop: Ctrl-C · reset: docker compose up (every start is fresh)");
    } else {
        println!("  stop: Ctrl-C · reset: cargo xtask judge (always starts fresh)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_ten_warns_and_older_stops() {
        assert!(npm_finding("11.20.0").is_none());
        assert!(npm_finding("12.0.0").is_none());
        let ten = npm_finding("10.9.2").expect("npm 10 is reported");
        assert!(ten.warning, "npm 10 boots with --legacy-peer-deps");
        let nine = npm_finding("9.8.1").expect("npm 9 is reported");
        assert!(!nine.warning, "npm 9 stops the boot");
        assert!(npm_finding("garbage").is_none());
    }

    #[test]
    fn majors_parse_node_npm_and_prose() {
        assert_eq!(major("v24.1.0"), Some(24));
        assert_eq!(major("11.2.0"), Some(11));
        assert_eq!(major("git version 2.47.1"), Some(2));
        assert_eq!(major(""), None);
    }
}
