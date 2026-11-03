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
//! probes and exits (CI uses it).
//!

use std::net::TcpListener;
use std::process::Command;

/// Free disk the first build needs (release target + node_modules +
/// both SPAs + the store), with headroom.
const MIN_FREE_GB: u64 = 6;
/// `web/package.json` engines.
const NODE_MIN_MAJOR: u64 = 24;
const NPM_MIN_MAJOR: u64 = 11;

/// One doctor finding: what failed and the exact command that fixes it.
struct Problem {
    what: String,
    fix: String,
}

/// Entry point (the builder wires the `judge` arm in `main.rs`).
pub fn run(args: &[String]) {
    let doctor_only = args.iter().any(|a| a == "--doctor");
    if let Err(e) = crate::up::apply_port_base(args) {
        eprintln!("judge: {e}");
        std::process::exit(2);
    }
    println!("judge: checking prerequisites (nothing is installed for you)");
    let problems = doctor();
    if !problems.is_empty() {
        eprintln!();
        eprintln!("judge: {} prerequisite(s) to fix first:", problems.len());
        for p in &problems {
            eprintln!("  ✗ {}", p.what);
            eprintln!("    fix: {}", p.fix);
        }
        eprintln!();
        eprintln!("Then run `cargo xtask judge` again.");
        std::process::exit(1);
    }
    println!("judge: all prerequisites ok");
    if doctor_only {
        return;
    }
    println!(
        "judge: first run builds both web apps and the release binaries — about 8-12 minutes; reruns take a minute or two"
    );
    crate::up::set_env_label("judge");
    let mut forwarded: Vec<String> = vec!["--fresh".to_string()];
    forwarded.extend(args.iter().filter(|a| *a != "--doctor").cloned());
    crate::up::run(&forwarded);
}

fn doctor() -> Vec<Problem> {
    let mut problems = Vec::new();
    match std::env::consts::OS {
        "macos" | "linux" => {}
        other => problems.push(Problem {
            what: format!("unsupported OS {other:?} (macOS or Linux only)"),
            fix: "run it on macOS or Linux (WSL2 works on Windows)".to_string(),
        }),
    }
    if version_of("git", &["--version"]).is_none() {
        problems.push(Problem {
            what: "git is not on PATH".to_string(),
            fix: "macOS: xcode-select --install · Debian/Ubuntu: sudo apt install git".to_string(),
        });
    }
    match version_of("node", &["--version"]) {
        None => problems.push(Problem {
            what: "Node.js is not on PATH".to_string(),
            fix: format!("install Node {NODE_MIN_MAJOR}+: nvm install {NODE_MIN_MAJOR} && nvm use {NODE_MIN_MAJOR}"),
        }),
        Some(v) if major(&v).is_some_and(|m| m < NODE_MIN_MAJOR) => problems.push(Problem {
            what: format!("Node {v} is older than {NODE_MIN_MAJOR}"),
            fix: format!("nvm install {NODE_MIN_MAJOR} && nvm use {NODE_MIN_MAJOR}"),
        }),
        Some(_) => {}
    }
    match version_of("npm", &["--version"]) {
        None => problems.push(Problem {
            what: "npm is not on PATH".to_string(),
            fix: format!("it ships with Node {NODE_MIN_MAJOR}+: nvm install {NODE_MIN_MAJOR}"),
        }),
        Some(v) if major(&v).is_some_and(|m| m < NPM_MIN_MAJOR) => problems.push(Problem {
            what: format!("npm {v} is older than {NPM_MIN_MAJOR}"),
            fix: format!("npm install -g npm@{NPM_MIN_MAJOR}"),
        }),
        Some(_) => {}
    }
    for port in [crate::up::public_port(), crate::up::admin_port()] {
        if TcpListener::bind(("127.0.0.1", port)).is_err() {
            problems.push(Problem {
                what: format!("port {port} is in use on 127.0.0.1"),
                fix: format!("stop whatever holds it: lsof -nP -iTCP:{port} -sTCP:LISTEN"),
            });
        }
    }
    if let Some(free) = free_gb(".")
        && free < MIN_FREE_GB
    {
        problems.push(Problem {
            what: format!("only {free} GB free here; the first build needs about {MIN_FREE_GB} GB"),
            fix: "free disk space, or clone into a roomier volume".to_string(),
        });
    }
    if std::path::Path::new("Cargo.toml").exists()
        && !std::path::Path::new("web/package.json").exists()
    {
        problems.push(Problem {
            what: "not at the repository root".to_string(),
            fix: "cd into the cloned repository, then cargo xtask judge".to_string(),
        });
    }
    problems
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
pub fn print_tour(public_port: u16, admin_port: u16) {
    println!();
    println!("judge tour (5 minutes; details in docs/judges.md):");
    println!("  1. http://localhost:{public_port}/apostille — press ⌘K, search \"Washington\"");
    println!(
        "  2. http://localhost:{admin_port}/config — paste the setup key above, enrol a passkey"
    );
    println!(
        "  3. Review: each Muse Spark draft shows its quote; accept a few with the dossier open in"
    );
    println!("     another tab — it updates live over WebSocket, no reload");
    println!("  4. the dossier: every accepted fact shows its quote, source and date one tap away");
    println!(
        "  5. Trace: \"Draft from this capture\" streams a model run (recorded unless --live)"
    );
    println!("  stop: Ctrl-C · reset: cargo xtask judge (always starts fresh)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn majors_parse_node_npm_and_prose() {
        assert_eq!(major("v24.1.0"), Some(24));
        assert_eq!(major("11.2.0"), Some(11));
        assert_eq!(major("git version 2.47.1"), Some(2));
        assert_eq!(major(""), None);
    }
}
