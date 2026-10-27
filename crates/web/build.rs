//! Build-time git SHA stamp (`BUILD_SHA`, short hash or `unknown`).
//!
//! `healthz`/`version` report `CARGO_PKG_VERSION+BUILD_SHA` so every
//! deploy is identifiable on the wire (which release is serving?).
//! Re-runs when HEAD moves: tracks `.git/HEAD` plus the ref file it
//! points at (or nothing extra on detached HEAD).

use std::process::Command;

fn main() {
    let git = std::path::Path::new(".git");
    println!("cargo:rerun-if-changed={}", git.join("HEAD").display());
    if let Ok(head) = std::fs::read_to_string(git.join("HEAD")) {
        // `ref: refs/heads/main` → also watch that ref file.
        if let Some(refpath) = head.trim().strip_prefix("ref: ") {
            println!("cargo:rerun-if-changed={}", git.join(refpath).display());
        }
    }
    // Worktrees (like our isolated builders) keep a `.git` FILE
    // pointing at the real dir; `git` itself resolves it, so only
    // the command's success matters, not the layout above.
    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_SHA={sha}");
}
