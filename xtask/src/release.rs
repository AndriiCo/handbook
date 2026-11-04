//! Rootless release tooling: `xtask deploy` + `xtask rollback`.
//!
//! Replaces the old shell ship script with tested Rust.
//! Runs ON the server as the unprivileged `hal` user: `hal` needs
//! group-write on `<root>/releases/` plus the pinned grants from
//! `xtask provision` (`systemctl restart`
//! among them — never stop/disable). Layout under `--root`
//! (default `/srv/apostille`):
//!
//! ```text
//! releases/<version>/{web,spa/,release.toml}
//! current -> releases/<version>
//! ```

use std::path::{Path, PathBuf};

/// Production releases root on the server.
pub const DEFAULT_ROOT: &str = "/srv/apostille";

/// Systemd unit restarted around the symlink swap.
pub const SERVICE: &str = "apostille";

/// Prod smoke URL (also validates the TLS chain).
pub const DEFAULT_SMOKE_URL: &str = "https://andrii.ai/apostille/healthz";

/// Smoke URL for the host at `root`: `xtask provision` records the
/// host's public name in `<root>/public-host` (the rehearsal container
/// is `rehearsal.localhost`); without that file, production's URL.
#[must_use]
pub fn default_smoke_url(root: &Path) -> String {
    std::fs::read_to_string(root.join("public-host"))
        .ok()
        .map(|text| text.trim().to_ascii_lowercase())
        .filter(|host| {
            !host.is_empty()
                && host.len() <= 200
                && host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        })
        .map_or_else(
            || DEFAULT_SMOKE_URL.to_string(),
            |host| format!("https://{host}/apostille/healthz"),
        )
}

/// Release dirs kept after a successful deploy (the live one plus the
/// newest 3); older ones are pruned oldest-first.
pub const MAX_RELEASES: usize = 4;

/// Migrations old code cannot run against: table rebuilds,
/// renames, drops, column type changes, or STRICT added to an
/// existing table. Purely additive migrations (new tables, new
/// nullable columns) are soft: old code never touches them.
///
/// Live registry (mirrors `handbook-store/migrations/`):
/// - 005 HARD: rebuilds every table (`*_new` + INSERT..SELECT +
///   DROP + RENAME) to add CHECKs/FKs; the capture rebuild is the
///   canonical example.
/// - 006 soft: two NEW STRICT tables, nothing existing changed.
/// - 007 soft: one new table + two ADD COLUMNs.
/// - p2_001 soft: separate `p2.sqlite`, untouched by old code.
/// - 012 HARD: rebuilds `fact` (`fact_new` + copy + DROP + RENAME) to
///   add the `service_notice` topic to its CHECK (P394).
/// - 013 listed although additive (ADD COLUMNs, new tables): every
///   older binary refuses a v13 store (`db::migrate` refuses a newer
///   version), so rolling back past it means restoring the backup.
///
/// When a new HARD migration lands, append its number here with
/// the reason — the rollback guard reads this table.
pub const HARD_MIGRATIONS: &[(i32, &str)] = &[
    (
        5,
        "005_constraints: table rebuilds adding CHECK/FK enforcement",
    ),
    (
        12,
        "012_service_notice: fact table rebuild widening the topic CHECK",
    ),
    (
        13,
        "013_authority_hidden: pre-013 binaries refuse a v13 store; restore the pre-deploy backup",
    ),
];

/// Per-release manifest, stored as `releases/<version>/release.toml`.
/// Written by [`deploy`], read by [`rollback`]'s guard. Field spec
/// with examples lives in the repo-root `release.toml`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReleaseManifest {
    /// Release dir name: git short sha (7..=64 lowercase hex).
    pub version: String,
    /// Full git sha the release was built from.
    pub sha: String,
    /// `handbook.sqlite` schema (`db::SCHEMA_VERSION` at build time).
    pub schema_version: i32,
    /// `p2.sqlite` schema (`db::P2_SCHEMA_VERSION` at build time).
    pub p2_schema_version: i32,
    /// Free text: which migrations this release crosses, data notes.
    pub migration_notes: String,
    /// False forces `--force` + `--backup-note` for any rollback
    /// away from this release, even across soft-only schemas.
    pub rollback_safe: bool,
}

/// Read and parse a `release.toml` manifest.
///
/// # Errors
///
/// Returns a message when the file is missing or not a valid manifest.
pub fn read_manifest(path: &Path) -> Result<ReleaseManifest, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
}

/// Serialize a manifest to `release.toml`.
///
/// # Errors
///
/// Returns a message when serialization or the write fails.
pub fn write_manifest(path: &Path, manifest: &ReleaseManifest) -> Result<(), String> {
    let text = toml::to_string_pretty(manifest).map_err(|e| format!("encode manifest: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

/// `git rev-parse HEAD` for the default manifest `sha`.
///
/// # Errors
///
/// Returns a message when git is unavailable or the tree has no HEAD.
pub fn git_head_sha() -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| format!("git rev-parse: {e}"))?;
    if !out.status.success() {
        return Err("git rev-parse HEAD failed (pass --sha explicitly)".to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Version names are git shas: 7..=64 lowercase hex. Anything else
/// exits before touching the tree (same rule as the ship script).
///
/// # Errors
///
/// Returns a message when the version is not 7..=64 hex chars.
pub fn validate_version(version: &str) -> Result<(), String> {
    let hex =
        version.len() >= 7 && version.len() <= 64 && version.bytes().all(|b| b.is_ascii_hexdigit());
    let lower = version.bytes().all(|b| !b.is_ascii_uppercase());
    if hex && lower {
        Ok(())
    } else {
        Err(format!(
            "bad version (want 7..=64 lowercase hex): {version}"
        ))
    }
}

/// Refuse a staged dir that cannot become a release: `web` and
/// `xtask` must be ELF binaries (script/zip/empty installs
/// rejected); `spa`, `spa-admin`, and `static` must be dirs.
/// `packs/` ships for reference and `evals/` for the A5 eval step
/// when staged (both optional).
///
/// # Errors
///
/// Returns a message naming the first missing/rejected member.
pub fn validate_release_src(src: &Path) -> Result<(), String> {
    for bin in ["web", "xtask"] {
        let path = src.join(bin);
        let magic = std::fs::read(&path).map_err(|_| format!("missing {}", path.display()))?;
        if magic.len() < 4 || magic[0..4] != [0x7f, b'E', b'L', b'F'] {
            return Err(format!("not an ELF binary: {}", path.display()));
        }
    }
    for dir in ["spa", "spa-admin", "static"] {
        if !src.join(dir).is_dir() {
            return Err(format!("missing {}", src.join(dir).display()));
        }
    }
    Ok(())
}

/// Live version from `<root>/current`, trusted only when the link
/// points at `<root>/releases/<hex>` (a hand-edited symlink must
/// never steer a rollback or a revert).
#[must_use]
pub fn read_current(root: &Path) -> Option<String> {
    let target = std::fs::read_link(root.join("current")).ok()?;
    let releases = root.join("releases");
    let rest = target.strip_prefix(&releases).ok()?;
    if rest.components().count() != 1 {
        return None;
    }
    let name = rest.to_str()?;
    validate_version(name).ok()?;
    Some(name.to_string())
}

/// Atomically point `<root>/current` at `releases/<version>`
/// (symlink + rename). Returns the previous live version, if the
/// old link was trustworthy (see [`read_current`]).
///
/// # Errors
///
/// Returns a message when the link cannot be created or renamed.
pub fn atomic_swap(root: &Path, version: &str) -> Result<Option<String>, String> {
    let prev = read_current(root);
    let tmp = root.join("current.tmp");
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(root.join("releases").join(version), &tmp)
        .map_err(|e| format!("symlink {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, root.join("current"))
        .map_err(|e| format!("rename {}: {e}", tmp.display()))?;
    Ok(prev)
}

/// Pick prune victims from release names oldest-first so `keep`
/// dirs survive in total: the live release is never a victim, the
/// rest go oldest-first.
#[must_use]
pub fn select_prune(names_oldest_first: &[String], live: &str, keep: usize) -> Vec<String> {
    let spare = names_oldest_first.len().saturating_sub(keep);
    names_oldest_first
        .iter()
        .filter(|name| name.as_str() != live)
        .take(spare)
        .cloned()
        .collect()
}

/// Prune `<root>/releases/` to [`MAX_RELEASES`] dirs, oldest-first
/// by mtime (name breaks ties), never the live release, only
/// hex-named dirs. Returns the pruned versions, oldest-first.
///
/// # Errors
///
/// Returns a message when the releases dir cannot be listed or a
/// victim cannot be removed.
pub fn prune_old_releases(root: &Path, live: &str) -> Result<Vec<String>, String> {
    let releases = root.join("releases");
    let mut entries: Vec<(std::time::SystemTime, String)> = Vec::new();
    let dir =
        std::fs::read_dir(&releases).map_err(|e| format!("list {}: {e}", releases.display()))?;
    for entry in dir {
        let entry = entry.map_err(|e| format!("read entry: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if validate_version(&name).is_err() || !entry.path().is_dir() {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        entries.push((mtime, name));
    }
    entries.sort();
    let names: Vec<String> = entries.into_iter().map(|(_, name)| name).collect();
    let victims = select_prune(&names, live, MAX_RELEASES);
    for victim in &victims {
        std::fs::remove_dir_all(releases.join(victim))
            .map_err(|e| format!("prune {victim}: {e}"))?;
    }
    Ok(victims)
}

/// Hard migrations in `(to_schema, from_schema]`: the ones a
/// rollback from `from_schema` to `to_schema` would cross. Empty
/// when `to_schema >= from_schema` (roll-forward, nothing crossed).
#[must_use]
pub fn hard_crossings(from_schema: i32, to_schema: i32) -> Vec<(i32, &'static str)> {
    if to_schema >= from_schema {
        return Vec::new();
    }
    HARD_MIGRATIONS
        .iter()
        .filter(|(n, _)| *n > to_schema && *n <= from_schema)
        .copied()
        .collect()
}

/// Reasons a rollback from `current` to `target` needs `--force` +
/// `--backup-note`: every hard migration crossed, plus the live
/// release's own `rollback_safe = false` verdict. Empty means safe.
#[must_use]
pub fn rollback_blockers(current: &ReleaseManifest, target: &ReleaseManifest) -> Vec<String> {
    if target.schema_version >= current.schema_version {
        return Vec::new();
    }
    let mut blockers: Vec<String> = hard_crossings(current.schema_version, target.schema_version)
        .into_iter()
        .map(|(n, reason)| format!("crosses hard migration {n} ({reason})"))
        .collect();
    if !current.rollback_safe {
        blockers.push(format!(
            "release {} is marked rollback-unsafe ({})",
            current.version, current.migration_notes
        ));
    }
    blockers
}

/// Enforce the rollback guard: refuse when [`rollback_blockers`]
/// is non-empty unless `force` is set AND `backup_note` names the
/// backup the author took (SQLite has no down-migrations; the note
/// is the audit trail, not a formality).
///
/// # Errors
///
/// Returns the blockers plus override instructions when the guard refuses.
pub fn check_rollback_guard(
    current: &ReleaseManifest,
    target: &ReleaseManifest,
    force: bool,
    backup_note: Option<&str>,
) -> Result<(), String> {
    let blockers = rollback_blockers(current, target);
    if blockers.is_empty() {
        return Ok(());
    }
    let noted = backup_note.is_some_and(|n| !n.trim().is_empty());
    if force && noted {
        return Ok(());
    }
    Err(format!(
        "rollback {} -> {} REFUSED (SQLite has no down-migrations):\n- {}\n\
         override: --force --backup-note '<db backup id/location>' \
         (author's explicit call only)",
        current.version,
        target.version,
        blockers.join("\n- ")
    ))
}

/// `sudo -n systemctl <action> apostille`. Only `restart` is used here,
/// and hal's pinned sudoers entry (`xtask/ops/wrappers/hal-apostille.sudoers`)
/// covers restart/reload/status/is-active — never stop/disable, and
/// `-n` fails fast instead of hanging on a password prompt.
///
/// # Errors
///
/// Returns a message when systemctl is missing or reports failure.
pub fn systemctl(action: &str) -> Result<(), String> {
    let out = std::process::Command::new("sudo")
        .args(["-n", "systemctl", action, SERVICE])
        .output()
        .map_err(|e| format!("sudo systemctl {action}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "systemctl {action} {SERVICE} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Smoke a health endpoint: `curl -fsS` must exit 0 AND the body
/// must contain `"status":"ok"` (whitespace-insensitive), matching
/// the `/apostille/healthz` contract.
///
/// # Errors
///
/// Returns a message when curl fails, is missing, or the body mismatches.
pub fn smoke(url: &str) -> Result<(), String> {
    let out = std::process::Command::new("curl")
        .args(["-fsS", "--max-time", "15", url])
        .output()
        .map_err(|_| "smoke: curl missing (install curl)".to_string())?;
    if !out.status.success() {
        return Err(format!("smoke {url}: curl exit {}", out.status));
    }
    let body: String = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .collect();
    if body.contains(r#""status":"ok""#) {
        Ok(())
    } else {
        Err(format!("smoke {url}: body lacks status ok"))
    }
}

/// Poll a health endpoint until it answers `"status":"ok"`: one
/// `curl -fsS --max-time 2` per try, `interval_ms` between tries,
/// at most `tries` tries. Replaces the old fixed settle sleeps: a
/// restarted service is used as soon as it answers instead of after
/// a blind wait.
///
/// # Errors
///
/// Returns a message when no try answers ok (curl missing counts
/// as an immediate error).
pub fn poll_health_with(url: &str, interval_ms: u64, tries: u32) -> Result<(), String> {
    let mut last = "no tries".to_string();
    for _ in 0..tries.max(1) {
        let out = std::process::Command::new("curl")
            .args(["-fsS", "--max-time", "2", url])
            .output()
            .map_err(|_| "health poll: curl missing (install curl)".to_string())?;
        if out.status.success() {
            let body: String = String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .collect();
            if body.contains(r#""status":"ok""#) {
                return Ok(());
            }
            last = "body lacks status ok".to_string();
        } else {
            last = format!("curl exit {}", out.status);
        }
        std::thread::sleep(std::time::Duration::from_millis(interval_ms));
    }
    Err(format!("health poll {url} never ok: {last}"))
}

/// [`poll_health_with`] with the longer settle budget:
/// every 250 ms, up to 15 s total.
pub fn poll_health(url: &str) -> Result<(), String> {
    poll_health_with(url, 250, 60)
}

/// Smoke a health endpoint AND pin the build: like [`smoke`], plus the
/// body must name `"version":"0.1.0+{sha7}"` (runbook-release §3). A
/// stale binary or orphan process serving the port fails the release
/// instead of shipping under the wrong stamp.
pub fn smoke_sha(url: &str, sha7: &str) -> Result<(), String> {
    smoke(url)?;
    let out = std::process::Command::new("curl")
        .args(["-fsS", "--max-time", "15", url])
        .output()
        .map_err(|_| "smoke: curl missing (install curl)".to_string())?;
    let body: String = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .collect();
    let want = format!(r#""version":"0.1.0+{sha7}""#);
    if body.contains(&want) {
        Ok(())
    } else {
        Err(format!("smoke {url}: build SHA does not name {sha7}"))
    }
}

/// Recursive copy of a dir tree (files + subdirs; symlinks not followed).
fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("mkdir {}: {e}", dst.display()))?;
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((from, to)) = stack.pop() {
        let entries =
            std::fs::read_dir(&from).map_err(|e| format!("list {}: {e}", from.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("read entry: {e}"))?;
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                std::fs::create_dir_all(&target)
                    .map_err(|e| format!("mkdir {}: {e}", target.display()))?;
                stack.push((entry.path(), target));
            } else {
                std::fs::copy(entry.path(), &target)
                    .map_err(|e| format!("copy {}: {e}", entry.path().display()))?;
            }
        }
    }
    Ok(())
}

/// Copy a validated staged release into `<root>/releases/<version>/`
/// and write its manifest (no swap, no restart). Shared by `deploy` and
/// the first install in `provision`.
pub fn install_release(
    root: &Path,
    src: &Path,
    version: &str,
    sha: &str,
    migration_notes: &str,
    rollback_safe: bool,
) -> Result<(), String> {
    let dst = root.join("releases").join(version);
    std::fs::create_dir_all(&dst).map_err(|e| format!("mkdir {}: {e}", dst.display()))?;
    for bin in ["web", "xtask"] {
        std::fs::copy(src.join(bin), dst.join(bin)).map_err(|e| format!("copy {bin}: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(dst.join(bin))
                .map_err(|e| format!("stat {bin}: {e}"))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(dst.join(bin), perms)
                .map_err(|e| format!("chmod {bin}: {e}"))?;
        }
    }
    for dir in ["spa", "spa-admin", "static"] {
        let _ = std::fs::remove_dir_all(dst.join(dir));
        copy_dir(&src.join(dir), &dst.join(dir))?;
    }
    if src.join("packs").is_dir() {
        let _ = std::fs::remove_dir_all(dst.join("packs"));
        copy_dir(&src.join("packs"), &dst.join("packs"))?;
    }
    if src.join("evals").is_dir() {
        let _ = std::fs::remove_dir_all(dst.join("evals"));
        copy_dir(&src.join("evals"), &dst.join("evals"))?;
    }
    write_manifest(
        &dst.join("release.toml"),
        &ReleaseManifest {
            version: version.to_string(),
            sha: sha.to_string(),
            schema_version: handbook_store::db::SCHEMA_VERSION,
            p2_schema_version: handbook_store::db::P2_SCHEMA_VERSION,
            migration_notes: migration_notes.to_string(),
            rollback_safe,
        },
    )?;

    Ok(())
}

/// Options for [`deploy`]. `packs_dir`/`expect_build` are `None`
/// exactly when the caller passed the matching `--skip-*` flag.
pub struct DeployOptions {
    /// Releases root (`/srv/apostille` on prod, tempdir in tests).
    pub root: PathBuf,
    /// Staged dir: `web` + `xtask` ELFs, `spa/` + `spa-admin/` +
    /// `static/` dirs, optional `packs/` (see [`validate_release_src`]).
    pub src: PathBuf,
    /// Release dir name (git short sha).
    pub version: String,
    /// Full git sha for the manifest.
    pub sha: String,
    /// Free-text migration notes for the manifest.
    pub migration_notes: String,
    /// Manifest `rollback_safe` flag (false needs `--rollback-unsafe`).
    pub rollback_safe: bool,
    /// Packs dir for the packs-validate gate (`None` skips).
    pub packs_dir: Option<PathBuf>,
    /// Expected build for the accept gate (`None` skips).
    pub expect_build: Option<i64>,
    /// Store for the accept gate.
    pub store: PathBuf,
    /// Audit actor for the accept row.
    pub actor: String,
    /// Run `systemctl restart` (false in tests).
    pub restart: bool,
    /// Health URL to smoke after the swap.
    pub smoke_url: String,
    /// Unused since the day gate was removed (deploys are instant
    /// per operator order): kept so callers and tests do not churn.
    /// `None` is the normal value.
    pub pt_midnight_secs: Option<i64>,
    /// Accepted as a silent no-op (`--allow-today-overlap`): kept so
    /// existing deploy lines keep working. The default is false;
    /// there is no env knob.
    pub allow_today_overlap: bool,
}

/// Ship one release: gates (packs-validate + accept), staged-set
/// validation, install + manifest, atomic `current` swap, restart,
/// smoke; on smoke failure the symlink flips back and the old
/// release restarts. Prunes to [`MAX_RELEASES`] on success only.
///
/// # Errors
///
/// Returns a message when any gate, install, restart, or smoke
/// step fails (after best-effort revert on smoke failure).
pub fn deploy(opts: &DeployOptions) -> Result<String, String> {
    validate_version(&opts.version)?;
    // No day gate: deploys are instant (operator order; the flag
    // below stays accepted as a silent no-op). `pt_midnight_secs`
    // is ignored for the same reason.
    let _ = (opts.allow_today_overlap, opts.pt_midnight_secs.as_ref());
    if let Some(dir) = &opts.packs_dir {
        // The whole tree (curated + world sets): a release must not
        // ship a junk world name or a slug two packs fight over.
        let errors = crate::packs::validate_tree(dir);
        if !errors.is_empty() {
            let details: Vec<String> = errors.iter().map(ToString::to_string).collect();
            return Err(format!("packs-validate refused:\n{}", details.join("\n")));
        }
    }
    if let Some(expect) = opts.expect_build {
        let store = handbook_store::store::Store::new(opts.store.clone());
        crate::accept::accept(&store, expect, &opts.actor)
            .map_err(|reason| format!("accept gate refused: {reason}"))?;
    }
    validate_release_src(&opts.src)?;

    install_release(
        &opts.root,
        &opts.src,
        &opts.version,
        &opts.sha,
        &opts.migration_notes,
        opts.rollback_safe,
    )?;

    let prev = atomic_swap(&opts.root, &opts.version)?;
    if opts.restart {
        systemctl("restart")?;
        // Settle by polling, not sleeping: `smoke_sha` below is the
        // verdict, so a still-starting service only costs its real
        // startup time here.
        if let Err(e) = poll_health(&opts.smoke_url) {
            eprintln!("deploy: settle poll: {e}");
        }
    }
    match smoke_sha(&opts.smoke_url, &opts.version) {
        Ok(()) => {
            let pruned = prune_old_releases(&opts.root, &opts.version)?;
            Ok(format!(
                "deployed {} (previous: {}, pruned: {})",
                opts.version,
                prev.as_deref().unwrap_or("none"),
                if pruned.is_empty() {
                    "none".to_string()
                } else {
                    pruned.join(",")
                }
            ))
        }
        Err(failure) => {
            revert_to_prev(&opts.root, prev.as_deref(), opts.restart, &opts.smoke_url);
            Err(format!("smoke FAILED for {}: {failure}", opts.version))
        }
    }
}

/// Best-effort revert after a failed smoke: flip `current` back to
/// `prev` (when trustworthy and its `web` still exists) and restart.
/// Without a valid prev the new release stays linked — hal's
/// sudoers cover restart, not stop, so manual rescue follows.
fn revert_to_prev(root: &Path, prev: Option<&str>, restart: bool, smoke_url: &str) {
    let usable = prev.filter(|p| root.join("releases").join(p).join("web").is_file());
    match usable {
        Some(p) => {
            if atomic_swap(root, p).is_err() {
                eprintln!("revert: FAILED to flip current back to {p}");
                return;
            }
            if restart {
                if systemctl("restart").is_err() {
                    eprintln!("revert: restart of {p} FAILED");
                    return;
                }
                if let Err(e) = poll_health(smoke_url) {
                    eprintln!("revert: settle poll: {e}");
                }
            }
            match smoke(smoke_url) {
                Ok(()) => eprintln!("revert: {p} live again"),
                Err(e) => eprintln!("revert: {p} SMOKE FAILED ({e}) - investigate"),
            }
        }
        None => eprintln!(
            "revert: no previous release - service left on the failed \
             version (restart manually after fixing)"
        ),
    }
}

/// Options for [`rollback`].
pub struct RollbackOptions {
    /// Releases root (`/srv/apostille` on prod, tempdir in tests).
    pub root: PathBuf,
    /// Target release dir name (must already exist).
    pub to: String,
    /// Override the rollback guard (needs `backup_note` too).
    pub force: bool,
    /// DB backup id/location (the author's audit trail).
    pub backup_note: Option<String>,
    /// Run `systemctl restart` (false in tests).
    pub restart: bool,
    /// Health URL to verify after the swap.
    pub smoke_url: String,
}

/// Roll back to an existing release: guard (hard-migration
/// crossings need `--force` + `--backup-note`), repoint `current`,
/// restart, verify; on verify failure the original link is
/// restored and restarted.
///
/// # Errors
///
/// Returns a message when the guard refuses, the target is
/// unknown, or any restart/verify step fails.
pub fn rollback(opts: &RollbackOptions) -> Result<String, String> {
    validate_version(&opts.to)?;
    let live = read_current(&opts.root)
        .ok_or_else(|| "no trustworthy live release (fix current by hand)".to_string())?;
    if live == opts.to {
        return Err(format!("already on {}", opts.to));
    }
    let releases = opts.root.join("releases");
    if !releases.join(&opts.to).join("web").is_file() {
        return Err(format!("unknown target release: {}", opts.to));
    }
    let current_manifest = read_manifest(&releases.join(&live).join("release.toml"))?;
    let target_manifest = read_manifest(&releases.join(&opts.to).join("release.toml"))?;
    check_rollback_guard(
        &current_manifest,
        &target_manifest,
        opts.force,
        opts.backup_note.as_deref(),
    )?;

    atomic_swap(&opts.root, &opts.to)?;
    if opts.restart {
        systemctl("restart")?;
        if let Err(e) = poll_health(&opts.smoke_url) {
            eprintln!("rollback: settle poll: {e}");
        }
    }
    match smoke_sha(&opts.smoke_url, &opts.to) {
        Ok(()) => Ok(format!("rolled back {live} -> {}", opts.to)),
        Err(failure) => {
            revert_to_prev(&opts.root, Some(&live), opts.restart, &opts.smoke_url);
            Err(format!("verify FAILED for {}: {failure}", opts.to))
        }
    }
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == &format!("--{name}"))
}

/// CLI entry: `xtask deploy --version <sha> --src <dir> [flags]`.
pub fn run_deploy(args: &[String]) {
    let version = crate::required(args, "version");
    let src = crate::required(args, "src");
    let sha = crate::flag(args, "sha").unwrap_or_else(|| {
        git_head_sha().unwrap_or_else(|e| {
            eprintln!("deploy: {e}");
            std::process::exit(1);
        })
    });
    let packs_dir = if has(args, "skip-packs") {
        None
    } else {
        Some(PathBuf::from(
            crate::flag(args, "packs").unwrap_or_else(|| "packs".to_string()),
        ))
    };
    let expect_build = if has(args, "skip-accept") {
        None
    } else {
        Some(
            crate::flag(args, "expect-build")
                .and_then(|b| b.parse().ok())
                .unwrap_or_else(|| {
                    eprintln!("missing required flag: --expect-build <n> (or --skip-accept)");
                    std::process::exit(2);
                }),
        )
    };
    let root = PathBuf::from(crate::flag(args, "root").unwrap_or_else(|| DEFAULT_ROOT.to_string()));
    let opts = DeployOptions {
        smoke_url: crate::flag(args, "smoke-url").unwrap_or_else(|| default_smoke_url(&root)),
        root,
        src: PathBuf::from(src),
        version,
        sha,
        migration_notes: crate::flag(args, "migration-notes").unwrap_or_default(),
        rollback_safe: !has(args, "rollback-unsafe"),
        packs_dir,
        expect_build,
        store: PathBuf::from(crate::flag(args, "store").unwrap_or_else(|| "store".to_string())),
        actor: crate::flag(args, "actor").unwrap_or_else(|| "xtask".to_string()),
        restart: !has(args, "no-restart"),
        // No day gate: deploys are instant per operator order, so PT
        // midnight is never read (the field stays for struct shape).
        pt_midnight_secs: None,
        allow_today_overlap: has(args, "allow-today-overlap"),
    };
    match deploy(&opts) {
        Ok(summary) => println!("{summary}"),
        Err(reason) => {
            eprintln!("deploy: {reason}");
            std::process::exit(1);
        }
    }
}

/// CLI entry: `xtask rollback --to <sha> [flags]`.
pub fn run_rollback(args: &[String]) {
    let root = PathBuf::from(crate::flag(args, "root").unwrap_or_else(|| DEFAULT_ROOT.to_string()));
    let opts = RollbackOptions {
        smoke_url: crate::flag(args, "smoke-url").unwrap_or_else(|| default_smoke_url(&root)),
        root,
        to: crate::required(args, "to"),
        force: has(args, "force"),
        backup_note: crate::flag(args, "backup-note"),
        restart: !has(args, "no-restart"),
    };
    match rollback(&opts) {
        Ok(summary) => println!("{summary}"),
        Err(reason) => {
            eprintln!("rollback: {reason}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke_url_follows_the_recorded_public_host() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(default_smoke_url(dir.path()), DEFAULT_SMOKE_URL);
        std::fs::write(dir.path().join("public-host"), "rehearsal.localhost\n").unwrap();
        assert_eq!(
            default_smoke_url(dir.path()),
            "https://rehearsal.localhost/apostille/healthz"
        );
        std::fs::write(dir.path().join("public-host"), "evil.test/x?y\n").unwrap();
        assert_eq!(
            default_smoke_url(dir.path()),
            DEFAULT_SMOKE_URL,
            "junk falls back"
        );
    }

    use super::*;
    use std::io::{Read, Write};

    fn manifest(version: &str, schema: i32, safe: bool) -> ReleaseManifest {
        ReleaseManifest {
            version: version.to_string(),
            sha: format!("{version}000000000000000000000000000000000"),
            schema_version: schema,
            p2_schema_version: 1,
            migration_notes: format!("test notes for {version}"),
            rollback_safe: safe,
        }
    }

    /// Fake releases root with `current` already on `live` (if given).
    /// Every version gets a stub `web` + manifest written by the caller.
    fn fake_root(live: Option<&str>) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("srv");
        std::fs::create_dir_all(root.join("releases")).unwrap();
        if let Some(v) = live {
            install_stub(&root, v, 7, true);
            atomic_swap(&root, v).unwrap();
        }
        (dir, root)
    }

    fn install_stub(root: &Path, version: &str, schema: i32, safe: bool) {
        let dst = root.join("releases").join(version);
        std::fs::create_dir_all(dst.join("spa")).unwrap();
        std::fs::write(dst.join("web"), [0x7f, b'E', b'L', b'F', 0]).unwrap();
        write_manifest(&dst.join("release.toml"), &manifest(version, schema, safe)).unwrap();
    }

    /// Staged full set: `web` + `xtask` (ELF magic only),
    /// `spa/` + `spa-admin/` + `static/` trees, optional `packs/`.
    fn staged(dir: &Path) -> PathBuf {
        let src = dir.join("staging");
        std::fs::create_dir_all(src.join("spa/assets")).unwrap();
        std::fs::create_dir_all(src.join("spa-admin")).unwrap();
        std::fs::create_dir_all(src.join("static")).unwrap();
        std::fs::write(src.join("web"), [0x7f, b'E', b'L', b'F', 1, 2, 3]).unwrap();
        std::fs::write(src.join("xtask"), [0x7f, b'E', b'L', b'F', 4, 5, 6]).unwrap();
        std::fs::write(src.join("spa/index.html"), "<html></html>").unwrap();
        std::fs::write(src.join("spa/assets/app.js"), "console.log(1)").unwrap();
        std::fs::write(src.join("spa-admin/index.html"), "<html>admin</html>").unwrap();
        std::fs::write(src.join("static/robots.txt"), "User-agent: *").unwrap();
        src
    }

    /// Versioned healthz server: serves two requests (smoke_sha
    /// fetches twice) with the given version string, then exits.
    fn versioned_server(version: &str) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/apostille/healthz",
            listener.local_addr().unwrap()
        );
        let body = format!(r#"{{"status":"ok","version":"{version}"}}"#);
        let handle = std::thread::spawn(move || {
            for _ in 0..2 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (url, handle)
    }

    /// Health stub: serves one `{"status":"ok"}` then exits.
    fn health_server_once() -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/apostille/healthz",
            listener.local_addr().unwrap()
        );
        let handle = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = r#"{"status":"ok"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (url, handle)
    }

    #[test]
    fn health_poll_passes_when_server_answers() {
        let (url, handle) = health_server_once();
        poll_health_with(&url, 10, 5).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn health_poll_fails_on_dead_port() {
        let url = dead_url();
        let err = poll_health_with(&url, 10, 3).expect_err("dead port");
        assert!(err.contains("never ok"), "{err}");
    }

    #[test]
    fn smoke_sha_pins_build() {
        let (url, handle) = versioned_server("0.1.0+abc1234");
        smoke_sha(&url, "abc1234").unwrap();
        handle.join().unwrap();
        let (url, handle) = versioned_server("0.1.0+deadbee");
        let err = smoke_sha(&url, "abc1234").expect_err("sha mismatch");
        assert!(err.contains("does not name"), "{err}");
        handle.join().unwrap();
    }

    /// A URL nothing listens on (listener bound on port 0, then dropped).
    fn dead_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/apostille/healthz",
            listener.local_addr().unwrap()
        );
        drop(listener);
        url
    }

    #[test]
    fn version_names_are_git_shas() {
        validate_version("abc1234").unwrap();
        validate_version(&"f".repeat(64)).unwrap();
        for bad in [
            "",
            "abc123",
            &"f".repeat(65),
            "ABC1234",
            "../../etc",
            "v1.2.3",
            "abc 1234",
        ] {
            validate_version(bad).expect_err(bad);
        }
    }

    #[test]
    fn manifest_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("release.toml");
        let want = manifest("abc1234", 7, true);
        write_manifest(&path, &want).unwrap();
        let got = read_manifest(&path).unwrap();
        assert_eq!(got, want);
        read_manifest(&dir.path().join("missing.toml")).unwrap_err();
        std::fs::write(&path, "version = [\n").unwrap();
        read_manifest(&path).unwrap_err();
    }

    #[test]
    fn staged_set_refuses_partial_members() {
        let dir = tempfile::tempdir().unwrap();
        let src = staged(dir.path());
        validate_release_src(&src).unwrap();
        std::fs::remove_dir_all(src.join("spa-admin")).unwrap();
        let err = validate_release_src(&src).unwrap_err();
        assert!(err.contains("spa-admin"), "{err}");
        std::fs::create_dir_all(src.join("spa-admin")).unwrap();
        std::fs::remove_file(src.join("xtask")).unwrap();
        let err = validate_release_src(&src).unwrap_err();
        assert!(err.contains("xtask"), "{err}");
    }

    #[test]
    fn root_spec_parses() {
        // Crate tests run with cwd = xtask/; the spec lives one up.
        let spec = read_manifest(Path::new("../release.toml")).unwrap();
        validate_version(&spec.version).unwrap();
        assert_eq!(spec.sha.len(), 40);
    }

    #[test]
    fn prune_keeps_newest_and_never_live() {
        let names: Vec<String> = (0..12).map(|n| format!("abc12{n:02}")).collect();
        // 12 releases, keep 10: prune 2 non-live, oldest-first.
        let victims = select_prune(&names, "abc1200", 10);
        assert_eq!(victims, vec!["abc1201".to_string(), "abc1202".to_string()]);
        // Live in the middle is spared even when old.
        let victims = select_prune(&names, "abc1205", 10);
        assert_eq!(victims, vec!["abc1200".to_string(), "abc1201".to_string()]);
        // Under the cap: nothing pruned.
        let few: Vec<String> = (0..10).map(|n| format!("abc12{n:02}")).collect();
        assert!(select_prune(&few, "abc1200", 10).is_empty());
        // Victims come oldest-first.
        let victims = select_prune(&names, "abc1211", 3);
        assert_eq!(victims.len(), 9);
        assert_eq!(victims[0], "abc1200");
        assert_eq!(victims[8], "abc1208");
    }

    #[test]
    fn guard_refuses_hard_crossings_only() {
        let v7 = manifest("aaaaaaa", 7, true);
        let v6 = manifest("bbbbbbb", 6, true);
        let v4 = manifest("ccccccc", 4, true);
        // 7 -> 6 crosses only soft 007: allowed.
        assert!(rollback_blockers(&v7, &v6).is_empty());
        // 7 -> 4 crosses hard 005: refused, naming the migration.
        let blockers = rollback_blockers(&v7, &v4);
        assert_eq!(blockers.len(), 1, "{blockers:?}");
        assert!(blockers[0].contains("005"), "{}", blockers[0]);
        // Roll-forward never crosses: allowed.
        assert!(rollback_blockers(&v6, &v7).is_empty());
        // Same schema: allowed.
        assert!(rollback_blockers(&v7, &v7).is_empty());
    }

    #[test]
    fn guard_force_needs_backup_note() {
        let v7 = manifest("aaaaaaa", 7, true);
        let v4 = manifest("ccccccc", 4, true);
        check_rollback_guard(&v7, &v4, false, None).unwrap_err();
        check_rollback_guard(&v7, &v4, true, None).unwrap_err();
        check_rollback_guard(&v7, &v4, true, Some("  ")).unwrap_err();
        check_rollback_guard(&v7, &v4, false, Some("backup-1")).unwrap_err();
        check_rollback_guard(&v7, &v4, true, Some("offsite nightly 2026-10-27")).unwrap();
    }

    #[test]
    fn guard_honors_unsafe_flag_on_soft_range() {
        let risky = manifest("aaaaaaa", 7, false);
        let v6 = manifest("bbbbbbb", 6, true);
        let blockers = rollback_blockers(&risky, &v6);
        assert_eq!(blockers.len(), 1, "{blockers:?}");
        assert!(blockers[0].contains("rollback-unsafe"), "{}", blockers[0]);
    }

    #[test]
    fn swap_returns_prev_and_rejects_escapes() {
        let (_dir, root) = fake_root(None);
        std::fs::create_dir_all(root.join("releases/aaaaaaa")).unwrap();
        std::fs::create_dir_all(root.join("releases/bbbbbbb")).unwrap();
        assert_eq!(atomic_swap(&root, "aaaaaaa").unwrap(), None);
        assert_eq!(read_current(&root).as_deref(), Some("aaaaaaa"));
        assert_eq!(
            atomic_swap(&root, "bbbbbbb").unwrap().as_deref(),
            Some("aaaaaaa")
        );
        // Hand-edited escape: trusted by nothing.
        std::fs::remove_file(root.join("current")).unwrap();
        std::os::unix::fs::symlink("/etc", root.join("current")).unwrap();
        assert_eq!(read_current(&root), None);
    }

    #[test]
    fn src_validation_rejects_non_elf_and_missing_spa() {
        let dir = tempfile::tempdir().unwrap();
        validate_release_src(&staged(dir.path())).unwrap();
        let bad = dir.path().join("bad");
        std::fs::create_dir_all(bad.join("spa")).unwrap();
        std::fs::write(bad.join("web"), b"#!/bin/sh\n").unwrap();
        validate_release_src(&bad).unwrap_err();
        let nospa = dir.path().join("nospa");
        std::fs::create_dir_all(&nospa).unwrap();
        std::fs::write(nospa.join("web"), [0x7f, b'E', b'L', b'F']).unwrap();
        validate_release_src(&nospa).unwrap_err();
    }

    #[test]
    fn deploy_installs_manifest_and_prunes_to_cap() {
        let (dir, root) = fake_root(None);
        // 12 pre-existing + the new one = prune the 9 oldest (13 -> 4).
        for n in 0..11 {
            install_stub(&root, &format!("abc12{n:02}"), 7, true);
        }
        install_stub(&root, "ddddddd", 7, true);
        atomic_swap(&root, "ddddddd").unwrap();
        let (url, server) = versioned_server("0.1.0+eeeeeee");
        let out = deploy(&DeployOptions {
            root: root.clone(),
            src: staged(dir.path()),
            version: "eeeeeee".to_string(),
            sha: "eeeeeee".to_string() + &"0".repeat(33),
            migration_notes: String::new(),
            rollback_safe: true,
            packs_dir: None,
            expect_build: None,
            store: dir.path().join("store"),
            actor: "test".to_string(),
            restart: false,
            smoke_url: url,
            pt_midnight_secs: None,
            allow_today_overlap: false,
        })
        .unwrap();
        server.join().unwrap();
        assert!(out.contains("deployed eeeeeee"), "{out}");
        assert_eq!(read_current(&root).as_deref(), Some("eeeeeee"));
        let installed = read_manifest(&root.join("releases/eeeeeee/release.toml")).unwrap();
        assert_eq!(installed.schema_version, handbook_store::db::SCHEMA_VERSION);
        assert!(installed.rollback_safe);
        assert!(root.join("releases/eeeeeee/spa/assets/app.js").is_file());
        assert!(root.join("releases/eeeeeee/spa-admin/index.html").is_file());
        assert!(root.join("releases/eeeeeee/static/robots.txt").is_file());
        assert!(root.join("releases/eeeeeee/xtask").is_file());
        let mut kept: Vec<String> = std::fs::read_dir(root.join("releases"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        kept.sort();
        assert_eq!(kept.len(), MAX_RELEASES, "{kept:?}");
        assert!(kept.contains(&"eeeeeee".to_string()));
        assert!(kept.contains(&"ddddddd".to_string()));
        assert!(!kept.contains(&"abc1200".to_string()));
    }

    #[test]
    fn deploy_reverts_link_on_failed_smoke() {
        let (dir, root) = fake_root(Some("ddddddd"));
        let err = deploy(&DeployOptions {
            root: root.clone(),
            src: staged(dir.path()),
            version: "eeeeeee".to_string(),
            sha: "e".repeat(40),
            migration_notes: String::new(),
            rollback_safe: true,
            packs_dir: None,
            expect_build: None,
            store: dir.path().join("store"),
            actor: "test".to_string(),
            restart: false,
            smoke_url: dead_url(),
            allow_today_overlap: false,
            pt_midnight_secs: None,
        })
        .unwrap_err();
        assert!(err.contains("smoke FAILED"), "{err}");
        assert_eq!(read_current(&root).as_deref(), Some("ddddddd"));
    }

    #[test]
    fn deploy_ignores_day_fields() {
        // No day gate: even a "spent" setup (fresh stub mtime, PT
        // midnight an hour ago) sails past gating — smoke then fails
        // on the dead URL and the link reverts to the previous live.
        let (dir, root) = fake_root(Some("ddddddd"));
        let err = deploy(&DeployOptions {
            root: root.clone(),
            src: staged(dir.path()),
            version: "eeeeeee".to_string(),
            sha: "e".repeat(40),
            migration_notes: String::new(),
            rollback_safe: true,
            packs_dir: None,
            expect_build: None,
            store: dir.path().join("store"),
            actor: "test".to_string(),
            restart: false,
            smoke_url: dead_url(),
            allow_today_overlap: false,
            pt_midnight_secs: Some(crate::unix_now() - 3600),
        })
        .unwrap_err();
        assert!(err.contains("smoke FAILED"), "{err}");
        assert_eq!(read_current(&root).as_deref(), Some("ddddddd"));
    }

    #[test]
    fn deploy_overlap_flag_is_silent_noop() {
        // `--allow-today-overlap` is accepted but changes nothing:
        // the same spent setup behaves exactly like the unflagged one.
        let (dir, root) = fake_root(Some("ddddddd"));
        let err = deploy(&DeployOptions {
            root: root.clone(),
            src: staged(dir.path()),
            version: "eeeeeee".to_string(),
            sha: "e".repeat(40),
            migration_notes: String::new(),
            rollback_safe: true,
            packs_dir: None,
            expect_build: None,
            store: dir.path().join("store"),
            actor: "test".to_string(),
            restart: false,
            smoke_url: dead_url(),
            pt_midnight_secs: Some(crate::unix_now() - 3600),
            allow_today_overlap: true,
        })
        .unwrap_err();
        assert!(err.contains("smoke FAILED"), "{err}");
        assert!(!err.contains("day gate"), "{err}");
        assert_eq!(read_current(&root).as_deref(), Some("ddddddd"));
    }

    #[test]
    fn rollback_repoints_and_guards() {
        let (_dir, root) = fake_root(None);
        install_stub(&root, "ccccccc", 4, true);
        install_stub(&root, "aaaaaaa", 7, true);
        atomic_swap(&root, "aaaaaaa").unwrap();
        let base = |to: &str| RollbackOptions {
            root: root.clone(),
            to: to.to_string(),
            force: false,
            backup_note: None,
            restart: false,
            smoke_url: dead_url(),
        };
        // Same version + unknown target rejected before the guard.
        rollback(&base("aaaaaaa")).unwrap_err();
        rollback(&base("fffffff")).unwrap_err();
        // Hard crossing refused without force + note.
        rollback(&base("ccccccc")).unwrap_err();
        // Forced rollback with a backup note succeeds against a live server.
        let (url, server) = versioned_server("0.1.0+ccccccc");
        let mut forced = base("ccccccc");
        forced.force = true;
        forced.backup_note = Some("offsite nightly 2026-10-27".to_string());
        forced.smoke_url = url;
        let out = rollback(&forced).unwrap();
        server.join().unwrap();
        assert!(out.contains("aaaaaaa -> ccccccc"), "{out}");
        assert_eq!(read_current(&root).as_deref(), Some("ccccccc"));
    }
}
