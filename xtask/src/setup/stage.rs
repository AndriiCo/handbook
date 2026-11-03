//! Build one exact commit into a release directory, the layout
//! `xtask deploy` and `xtask provision` install from:
//!
//! ```text
//! target/setup/<sha7>-<triple>/
//!   web  xtask  SHA  apostille.service
//!   spa/  spa-admin/  static/  packs/  evals/
//!   units/*.service|timer   scripts/ops-deploy.sh
//! ```
//!
//! The build runs in a throwaway worktree (`../hb-setup-<sha7>`), so a
//! dirty checkout never ships. Brand files reach the web prebuild only
//! when a recorded licence acceptance hashes them ([`super::brand`]). A
//! staged directory whose `SHA` and `BRAND` list match and whose
//! binaries pass the hardening check is reused.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::build::{self, Arch};
use super::plan::release_dir;

fn sh(cmd: &mut Command, what: &str) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("{what}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed"))
    }
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Full SHA of a committed revision.
pub fn resolve_sha(repo: &Path, rev: &str) -> Result<String, String> {
    let sha = git(
        repo,
        &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
    )?;
    if sha.len() != 40 {
        return Err(format!("{rev} did not resolve to a full SHA"));
    }
    Ok(sha)
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("mkdir {}: {e}", to.display()))?;
    // `cp -R src/. dst` copies contents, the same on BSD and GNU cp.
    sh(
        Command::new("cp").arg("-R").arg(from.join(".")).arg(to),
        &format!("copy {}", from.display()),
    )
}

fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("copy {}: {e}", from.display()))
}

/// Stage `sha` for `arch` with the licensed `brand` files; returns the
/// release directory.
pub fn stage_release(
    repo: &Path,
    sha: &str,
    arch: Arch,
    brand: &[PathBuf],
    log: &dyn Fn(String),
) -> Result<PathBuf, String> {
    let version = &sha[..7];
    let out = repo.join(release_dir(version, arch));
    let brand_list = brand
        .iter()
        .map(|p| {
            let bytes = std::fs::read(p).unwrap_or_default();
            format!(
                "{} {}\n",
                super::brand::sha256_hex(&bytes),
                p.file_name().unwrap_or_default().to_string_lossy()
            )
        })
        .collect::<String>();
    if std::fs::read_to_string(out.join("SHA")).is_ok_and(|s| s.trim() == sha)
        && std::fs::read_to_string(out.join("BRAND")).unwrap_or_default() == brand_list
        && build::harden_check(&out.join("web")).is_ok()
        && build::harden_check(&out.join("xtask")).is_ok()
    {
        log(format!("reusing {}", out.display()));
        return Ok(out);
    }
    let host = build::host_triple()?;
    let sccache = build::have("sccache");
    let plan = build::plan(arch, &host, sccache);
    if plan.zig && !build::have("zig") {
        return Err("cross-building needs zig 0.17 (brew install zig)".into());
    }
    log(format!(
        "{} build for {}{}",
        if plan.zig {
            "cargo zigbuild"
        } else {
            "cargo build"
        },
        arch.triple(),
        if sccache { " with sccache" } else { "" }
    ));
    let wt = repo
        .parent()
        .ok_or("repo has no parent directory")?
        .join(format!("hb-setup-{version}"));
    if wt.exists() {
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&wt)
            .current_dir(repo)
            .status();
    }
    git(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            &wt.display().to_string(),
            sha,
        ],
    )?;
    // Only licensed files reach the build (copied out of private/).
    let brand_dir = repo.join(format!("target/setup/brand-{version}"));
    let _ = std::fs::remove_dir_all(&brand_dir);
    std::fs::create_dir_all(&brand_dir).map_err(|e| e.to_string())?;
    for path in brand {
        copy_file(path, &brand_dir.join(path.file_name().unwrap_or_default()))?;
    }
    log(format!("{} licensed brand file(s)", brand.len()));
    let result = build_in(repo, &wt, sha, &plan, &out, &brand_dir, log);
    let _ = std::fs::remove_dir_all(&brand_dir);
    if result.is_ok() {
        std::fs::write(out.join("BRAND"), &brand_list).map_err(|e| e.to_string())?;
    }
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(&wt)
        .current_dir(repo)
        .status();
    result.map(|()| out)
}

fn build_in(
    repo: &Path,
    wt: &Path,
    sha: &str,
    plan: &build::BuildPlan,
    out: &Path,
    brand: &Path,
    log: &dyn Fn(String),
) -> Result<(), String> {
    // Same lockfile: reuse the checkout's node_modules (fast); else a clean install.
    let same_lock = Command::new("git")
        .args(["diff", "--quiet", sha, "--", "web/package-lock.json"])
        .current_dir(repo)
        .status()
        .is_ok_and(|s| s.success());
    if same_lock && repo.join("web/node_modules").is_dir() {
        std::os::unix::fs::symlink(repo.join("web/node_modules"), wt.join("web/node_modules"))
            .map_err(|e| format!("link node_modules: {e}"))?;
    } else {
        log("npm ci in the worktree".into());
        sh(
            Command::new("npm")
                .args(["ci", "--no-audit", "--no-fund"])
                .current_dir(wt.join("web")),
            "npm ci",
        )?;
    }
    let bins = build::cargo_release(plan, wt, &repo.join("target"))?;
    log("binaries hardened: PIE, NX stack, no W+X, full RELRO, stack protector".into());
    for script in ["build", "build:admin"] {
        log(format!("npm run {script}"));
        let mut cmd = Command::new("npm");
        cmd.args(["run", script])
            .current_dir(wt.join("web"))
            .env("BRAND_DIR", brand)
            .env("BRAND_FONT_DIR", brand);
        sh(&mut cmd, &format!("npm run {script}"))?;
    }
    if out.exists() {
        std::fs::remove_dir_all(out).map_err(|e| format!("clear {}: {e}", out.display()))?;
    }
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    for bin in ["web", "xtask"] {
        copy_file(&bins.join(bin), &out.join(bin))?;
    }
    copy_dir(&wt.join("web/build"), &out.join("spa"))?;
    copy_dir(&wt.join("web/build-admin"), &out.join("spa-admin"))?;
    copy_dir(&wt.join("crates/web/static"), &out.join("static"))?;
    copy_dir(&wt.join("packs"), &out.join("packs"))?;
    copy_file(
        &wt.join("crates/instruct/packs/general.toml"),
        &out.join("packs/general.toml"),
    )?;
    copy_dir(&wt.join("evals"), &out.join("evals"))?;
    copy_file(
        &wt.join("docs/ops/apostille.service"),
        &out.join("apostille.service"),
    )?;
    std::fs::create_dir_all(out.join("units")).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(wt.join("docs/ops")).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if name.starts_with("apostille") && (name.ends_with(".service") || name.ends_with(".timer"))
        {
            copy_file(&path, &out.join("units").join(&name))?;
        }
    }
    std::fs::create_dir_all(out.join("scripts")).map_err(|e| e.to_string())?;
    copy_file(
        &wt.join("xtask/ops/ops-deploy.sh"),
        &out.join("scripts/ops-deploy.sh"),
    )?;
    std::fs::write(out.join("SHA"), format!("{sha}\n")).map_err(|e| e.to_string())?;
    crate::release::validate_release_src(out)?;
    log(format!("staged {}", out.display()));
    Ok(())
}
