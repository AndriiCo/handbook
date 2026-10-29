//! Build-time git SHA stamp (`BUILD_SHA`, short hash or `unknown`).
//!
//! `healthz`/`version` report `CARGO_PKG_VERSION+BUILD_SHA` so every
//! deploy is identifiable on the wire (which release is serving?).
//! Re-runs when HEAD moves: tracks `.git/HEAD` plus the ref file it
//! points at (or nothing extra on detached HEAD).

use std::process::Command;

fn main() {
    // Build scripts run with CWD = package dir, where `.git`
    // does NOT exist — resolve the workspace `.git` via
    // CARGO_MANIFEST_DIR or the stamp never re-runs (every
    // deploy reports the same stale SHA).
    let manifest_dir =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    // Walk up to the dir holding `.git` (workspace root; the
    // crate may sit several levels down, e.g. `crates/web`).
    let workspace = manifest_dir
        .ancestors()
        .find(|a| a.join(".git").exists())
        .expect("workspace root with .git")
        .to_path_buf();
    let mut git_dir = workspace.join(".git");
    // Worktrees (like our isolated builders) keep a `.git` FILE
    // pointing at the real dir.
    if git_dir.is_file()
        && let Ok(content) = std::fs::read_to_string(&git_dir)
        && let Some(gitdir) = content.trim().strip_prefix("gitdir: ")
    {
        git_dir = workspace.join(gitdir);
    }
    println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    if let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD")) {
        // `ref: refs/heads/main` → also watch that ref file.
        if let Some(refpath) = head.trim().strip_prefix("ref: ") {
            println!("cargo:rerun-if-changed={}", git_dir.join(refpath).display());
        }
    }
    let sha = Command::new("git")
        .current_dir(&workspace)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_SHA={sha}");
}
