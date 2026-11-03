//! `xtask backup` + `xtask backup-verify`: nightly backups (drill 3).
//!
//! Spec: `docs/ops/prod.md` (SQLite backup API — never raw-copy a
//! WAL-mode DB — + `tar` of the file dirs, offsite, 7 daily + 4
//! weekly). `backup-verify` is the drill-3 engine: restore a daily
//! into a scratch dir and prove it boots-worthy, never over live data.
//!
//! Trust boundaries (prompt-083 SEC review): `backup` needs no KEK
//! (config rows are ciphertext, TLS keys are files) and writes
//! nothing to the live store; `backup-verify` without a KEK checks
//! structure only (`integrity_check`, versions, blob bytes, sums) —
//! sealed-row validation happens in drill 3's KEK-holding step.
//! The local backup tree relies on powered-off FDE, same as the
//! live store; ciphertext outside FDE exists only offsite (age).

use std::path::{Path, PathBuf};

/// Backups root on prod (compiled default, same style as
/// [`crate::release::DEFAULT_ROOT`]; override with `--dest`).
pub const DEFAULT_DEST: &str = "/var/lib/apostille-backups";

/// Retention: 7 daily + 4 weekly (`docs/ops/prod.md`).
const KEEP_DAILY: usize = 7;
const KEEP_WEEKLY: usize = 4;

/// File dirs captured by `files.tar.gz` (only those present are
/// archived; the two `*.sqlite` files travel via the backup API,
/// never via tar).
const TAR_MEMBERS: &[&str] = &[
    "blob",
    "warc",
    "mbox",
    "text",
    "acme",
    "tls",
    "admin-certs",
    "forms",
];

/// One database file in the backup set.
struct DbSpec {
    file: &'static str,
    version: i32,
    p2: bool,
}

const DBS: &[DbSpec] = &[
    DbSpec {
        file: "handbook.sqlite",
        version: handbook_store::db::SCHEMA_VERSION,
        p2: false,
    },
    DbSpec {
        file: "p2.sqlite",
        version: handbook_store::db::P2_SCHEMA_VERSION,
        p2: true,
    },
];

/// What [`backup_store`] produced (for the operator-facing summary).
struct BackupReport {
    id: String,
    files: Vec<(String, String)>,
    pruned_daily: Vec<String>,
    promoted_weekly: Option<String>,
    pruned_weekly: Vec<String>,
}

pub fn run(args: &[String]) {
    let store_root = crate::flag(args, "store").unwrap_or_else(|| "store".to_string());
    let dest_root = crate::flag(args, "dest").unwrap_or_else(|| DEFAULT_DEST.to_string());
    match backup_store(
        Path::new(&store_root),
        Path::new(&dest_root),
        crate::unix_now(),
    ) {
        Ok(report) => {
            println!("backup id: {}", report.id);
            // Per-file hashes to stdout: journald keeps them as a
            // second channel beside the backup tree (bitrot vs
            // tamper triage; see A2 in the prompt-083 SEC review).
            for (name, sha) in &report.files {
                println!("sha256 {sha}  {name}");
            }
            if let Some(weekly) = report.promoted_weekly {
                println!("promoted to weekly: {weekly}");
            }
            for victim in report.pruned_daily {
                println!("pruned daily: {victim}");
            }
            for victim in report.pruned_weekly {
                println!("pruned weekly: {victim}");
            }
        }
        Err(e) => {
            eprintln!("backup: {e}");
            std::process::exit(1);
        }
    }
}

/// Copy both databases via the SQLite backup API plus a tar of the
/// file dirs into `<dest>/daily/<YYYY-MM-DD>/`, verify the copies
/// before keeping them, then enforce retention. Reads the live store
/// only (no migrate, no audit write — the timer journal records the
/// run); refuses to create anything under the live store root.
///
/// # Errors
///
/// Returns a message when the store is missing a database, a copy or
/// its verification fails, tar fails, or retention cannot be enforced.
fn backup_store(
    store_root: &Path,
    dest_root: &Path,
    now_unix: i64,
) -> Result<BackupReport, String> {
    for spec in DBS {
        if !store_root.join(spec.file).is_file() {
            return Err(format!(
                "missing {} (refusing to back up a partial store)",
                store_root.join(spec.file).display()
            ));
        }
    }
    let daily_dir = dest_root.join("daily");
    std::fs::create_dir_all(&daily_dir)
        .map_err(|e| format!("create {}: {e}", daily_dir.display()))?;
    let staging = make_staging(dest_root)?;
    let outcome = backup_into(store_root, &staging, now_unix);
    // Publish-or-clean: a failed run leaves no half daily behind.
    let report = match outcome {
        Ok(report) => report,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    let _ = std::fs::remove_dir_all(&staging);
    Ok(report)
}

/// Create a fresh unique staging dir under `dest_root`
/// (`.staging.<pid>.<rand>`, 0700, `create_new` semantics so a
/// pre-created path or symlink is never followed or merged with —
/// same discipline as blob staging in `handbook-store`). Stale
/// staging dirs (>24h, from killed runs) are swept first; anything
/// younger is left alone so an overlapping run is never sabotaged.
fn make_staging(dest_root: &Path) -> Result<PathBuf, String> {
    sweep_stale_staging(dest_root)?;
    for _ in 0..100 {
        let name = format!(
            ".staging.{}.{:016x}",
            std::process::id(),
            rand::random::<u64>()
        );
        match std::fs::create_dir(dest_root.join(&name)) {
            Ok(()) => {
                let staging = dest_root.join(&name);
                restrict(&staging, true)?;
                return Ok(staging);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("create staging: {e}")),
        }
    }
    Err("could not create staging dir".to_string())
}

/// Remove `.staging.*` dirs older than 24h (killed runs). Anything
/// younger — or with an unreadable mtime — is left alone.
fn sweep_stale_staging(dest_root: &Path) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dest_root).map_err(|e| format!("list {}: {e}", dest_root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read entry: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(".staging.") || !entry.path().is_dir() {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|mtime| std::time::SystemTime::now().duration_since(mtime).ok())
            .is_some_and(|age| age.as_secs() > 86_400);
        if stale {
            std::fs::remove_dir_all(entry.path())
                .map_err(|e| format!("sweep stale {name}: {e}"))?;
        }
    }
    Ok(())
}

/// Stage, verify, and publish one backup; retention included. The
/// caller owns staging-dir cleanup on error.
fn backup_into(store_root: &Path, staging: &Path, now_unix: i64) -> Result<BackupReport, String> {
    let store = handbook_store::store::Store::new(store_root);
    for spec in DBS {
        copy_db(&store, spec, &staging.join(spec.file))?;
    }
    tar_files(store_root, &staging.join("files.tar.gz"))?;
    let (year, month, day) = civil_date(now_unix);
    let id = format!("{year:04}-{month:02}-{day:02}");
    let mut files = Vec::new();
    for name in ["handbook.sqlite", "p2.sqlite", "files.tar.gz"] {
        let bytes =
            std::fs::read(staging.join(name)).map_err(|e| format!("read staged {name}: {e}"))?;
        files.push((name, bytes.len(), handbook_store::store::sha256_hex(&bytes)));
    }
    let manifest = serde_json::json!({
        "backup_date": id,
        "created_unix": now_unix,
        "handbook_user_version": DBS[0].version,
        "p2_user_version": DBS[1].version,
        "files": files.iter().map(|(name, bytes, sha)| {
            serde_json::json!({"name": name, "bytes": bytes, "sha256": sha})
        }).collect::<Vec<_>>(),
    });
    let manifest_text =
        serde_json::to_string_pretty(&manifest).map_err(|e| format!("manifest json: {e}"))?;
    std::fs::write(staging.join("manifest.json"), &manifest_text)
        .map_err(|e| format!("write manifest: {e}"))?;
    let mut sums = String::new();
    for (name, _, sha) in &files {
        sums.push_str(&format!("{sha}  {name}\n"));
    }
    std::fs::write(staging.join("sha256sums"), &sums).map_err(|e| format!("write sums: {e}"))?;
    for name in [
        "handbook.sqlite",
        "p2.sqlite",
        "files.tar.gz",
        "manifest.json",
        "sha256sums",
    ] {
        restrict(&staging.join(name), false)?;
    }
    let dest_root = staging.parent().expect("staging always has a parent");
    let published = dest_root.join("daily").join(&id);
    if published.exists() {
        // Never overwrite a published daily: a same-day rerun must
        // remove today's dir explicitly first (mirrors the
        // never-newest retention discipline).
        return Err(format!(
            "daily {id} already exists (remove it to re-run today)"
        ));
    }
    fsync_staging(staging)?;
    std::fs::rename(staging, &published).map_err(|e| format!("publish {id}: {e}"))?;
    let pruned_daily = prune_backups(&dest_root.join("daily"), KEEP_DAILY)?;
    let mut promoted_weekly = None;
    let mut pruned_weekly = Vec::new();
    if is_sunday(now_unix) {
        let weekly_dir = dest_root.join("weekly");
        std::fs::create_dir_all(&weekly_dir)
            .map_err(|e| format!("create {}: {e}", weekly_dir.display()))?;
        let target = weekly_dir.join(&id);
        if !target.exists() {
            copy_dir(&published, &target)?;
            promoted_weekly = Some(id.clone());
        }
        pruned_weekly = prune_backups(&weekly_dir, KEEP_WEEKLY)?;
    }
    Ok(BackupReport {
        id,
        files: files
            .iter()
            .map(|(name, _, sha)| ((*name).to_string(), sha.clone()))
            .collect(),
        pruned_daily,
        promoted_weekly,
        pruned_weekly,
    })
}

/// Flush staged files and the staging dir itself so a power loss at
/// publish time cannot leave torn bytes behind a matching checksum.
fn fsync_staging(staging: &Path) -> Result<(), String> {
    for name in [
        "handbook.sqlite",
        "p2.sqlite",
        "files.tar.gz",
        "manifest.json",
        "sha256sums",
    ] {
        let file = std::fs::File::open(staging.join(name))
            .map_err(|e| format!("open staged {name}: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("fsync staged {name}: {e}"))?;
    }
    let dir = std::fs::File::open(staging).map_err(|e| format!("open staging dir: {e}"))?;
    dir.sync_all()
        .map_err(|e| format!("fsync staging dir: {e}"))?;
    Ok(())
}

/// Copy one live database to `dst` via the SQLite backup API (safe
/// on a WAL-mode DB under a running writer), then verify the copy:
/// `integrity_check` must be `ok` and `user_version` must match the
/// migrating version (a version drift means the copy is not
/// boot-worthy).
fn copy_db(store: &handbook_store::store::Store, spec: &DbSpec, dst: &Path) -> Result<(), String> {
    let src = if spec.p2 {
        handbook_store::db::open_p2(store)
    } else {
        handbook_store::db::open(store)
    }
    .map_err(|e| format!("open live {}: {e}", spec.file))?;
    let mut to = rusqlite::Connection::open(dst).map_err(|e| format!("open {dst:?}: {e}"))?;
    let backup = rusqlite::backup::Backup::new(&src, &mut to)
        .map_err(|e| format!("backup {}: {e}", spec.file))?;
    backup
        .run_to_completion(100, std::time::Duration::ZERO, None)
        .map_err(|e| format!("backup {}: {e}", spec.file))?;
    drop(backup);
    drop(to);
    drop(src);
    check_db_copy(dst, spec)
}

/// Verify a backup-API copy: opens, `integrity_check` is `ok`,
/// `user_version` matches the expected schema.
fn check_db_copy(dst: &Path, spec: &DbSpec) -> Result<(), String> {
    let conn = rusqlite::Connection::open(dst).map_err(|e| format!("reopen {dst:?}: {e}"))?;
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|e| format!("integrity_check {}: {e}", spec.file))?;
    if integrity != "ok" {
        return Err(format!("integrity_check {}: {integrity}", spec.file));
    }
    let version: i32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| format!("user_version {}: {e}", spec.file))?;
    if version != spec.version {
        return Err(format!(
            "{} user_version {version} (want {})",
            spec.file, spec.version
        ));
    }
    Ok(())
}

/// Archive the present file dirs from `store_root` into `archive`
/// with system tar (precedent: xtask already shells out to curl,
/// git, and systemctl). Blob temp files (`.tmp-*`, torn mid-write
/// by design) are excluded; the `*.sqlite` files are never members.
fn tar_files(store_root: &Path, archive: &Path) -> Result<(), String> {
    let members: Vec<&str> = TAR_MEMBERS
        .iter()
        .copied()
        .filter(|m| store_root.join(m).is_dir())
        .collect();
    let mut cmd = std::process::Command::new("tar");
    cmd.arg("-czf")
        .arg(archive)
        .arg("-C")
        .arg(store_root)
        .arg("--exclude=.tmp-*");
    if members.is_empty() {
        cmd.arg("--files-from").arg("/dev/null");
    } else {
        cmd.args(&members);
    }
    let out = cmd.output().map_err(|e| format!("run tar: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "tar: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Restrict a staged path to owner-only (dir: 0700, file: 0600):
/// the backup holds customer PII (`p2.sqlite`) and the TLS key
/// (`acme/`), so umask alone is not trusted.
fn restrict(path: &Path, dir: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = if dir { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("chmod {}: {e}", path.display()))
}

/// Prune `<dir>/` to `keep` date-named dirs, oldest-first by name
/// (`YYYY-MM-DD` sorts chronologically), never the newest (same
/// never-newest discipline as [`crate::release::select_prune`]).
/// Returns the pruned names, oldest-first.
fn prune_backups(dir: &Path, keep: usize) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("list {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read entry: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() && is_backup_id(&name) {
            names.push(name);
        }
    }
    names.sort();
    let live = names.last().cloned().unwrap_or_default();
    let victims = crate::release::select_prune(&names, &live, keep);
    for victim in &victims {
        std::fs::remove_dir_all(dir.join(victim)).map_err(|e| format!("prune {victim}: {e}"))?;
    }
    Ok(victims)
}

/// A backup id is a `YYYY-MM-DD` dir name (exactly what
/// [`backup_into`] publishes; anything else is left alone).
fn is_backup_id(name: &str) -> bool {
    name.len() == 10
        && name.as_bytes()[4] == b'-'
        && name.as_bytes()[7] == b'-'
        && name
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
}

/// Recursive copy of a dir tree (files + subdirs; symlinks not
/// followed — same contract as the release installer).
fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("create {}: {e}", dst.display()))?;
    let entries = std::fs::read_dir(src).map_err(|e| format!("list {}: {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read entry: {e}"))?;
        let target = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)
                .map_err(|e| format!("copy {}: {e}", entry.path().display()))?;
        }
    }
    Ok(())
}

pub fn run_verify(args: &[String]) {
    let backup = crate::required(args, "backup");
    let scratch = crate::required(args, "scratch");
    match verify_backup(Path::new(&backup), Path::new(&scratch)) {
        Ok(id) => println!("backup-verify: {id} restores green"),
        Err(e) => {
            eprintln!("backup-verify: {e}");
            std::process::exit(1);
        }
    }
}

/// Restore `<backup>/` into `<scratch>/data/` and prove it
/// boots-worthy: manifest parses and names the dir, `sha256sums`
/// recompute clean, both databases pass `integrity_check` +
/// `user_version` and a no-op migrate. Returns the backup id.
///
/// The scratch dir must be missing (created) or an empty dir —
/// anything else is refused, never deleted. The tar is treated as
/// untrusted input even though `backup` produced it (bitrot, a
/// tampered offsite copy): members are listed and rejected before
/// extraction, and the extracted tree must contain no symlinks and
/// stay inside scratch.
///
/// # Errors
///
/// Returns a message when the backup is incomplete or tampered, the
/// scratch dir is unsafe, extraction is refused, or a restored
/// database fails its checks.
fn verify_backup(backup_dir: &Path, scratch: &Path) -> Result<String, String> {
    if scratch.exists() {
        if !scratch.is_dir() {
            return Err(format!("scratch {} is not a dir", scratch.display()));
        }
        let empty = std::fs::read_dir(scratch)
            .map_err(|e| format!("list scratch: {e}"))?
            .next()
            .is_none();
        if !empty {
            return Err(format!(
                "scratch {} is not empty (refusing to mix restores)",
                scratch.display()
            ));
        }
    } else {
        std::fs::create_dir_all(scratch).map_err(|e| format!("create scratch: {e}"))?;
    }
    let manifest_text = std::fs::read_to_string(backup_dir.join("manifest.json"))
        .map_err(|e| format!("read manifest: {e}"))?;
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_text).map_err(|e| format!("parse manifest: {e}"))?;
    let dir_name = backup_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let manifest_date = manifest
        .get("backup_date")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if manifest_date != dir_name {
        return Err(format!(
            "manifest backup_date {manifest_date:?} names a different dir than {dir_name:?}"
        ));
    }
    check_sums(backup_dir)?;
    let data = scratch.join("data");
    std::fs::create_dir_all(&data).map_err(|e| format!("create data dir: {e}"))?;
    for spec in DBS {
        std::fs::copy(backup_dir.join(spec.file), data.join(spec.file))
            .map_err(|e| format!("restore {}: {e}", spec.file))?;
    }
    extract_files(&backup_dir.join("files.tar.gz"), &data)?;
    let store = handbook_store::store::Store::new(&data);
    let conn =
        handbook_store::db::open(&store).map_err(|e| format!("open restored main db: {e}"))?;
    handbook_store::db::migrate(&conn).map_err(|e| format!("migrate restored main db: {e}"))?;
    check_db_copy(&data.join("handbook.sqlite"), &DBS[0])?;
    let p2 =
        handbook_store::db::open_p2(&store).map_err(|e| format!("open restored p2 db: {e}"))?;
    handbook_store::db::migrate_p2(&p2).map_err(|e| format!("migrate restored p2 db: {e}"))?;
    check_db_copy(&data.join("p2.sqlite"), &DBS[1])?;
    Ok(dir_name)
}

/// Recompute every `sha256sums` line against the backup dir. Any
/// mismatch, missing file, or malformed line fails closed.
fn check_sums(backup_dir: &Path) -> Result<(), String> {
    let sums = std::fs::read_to_string(backup_dir.join("sha256sums"))
        .map_err(|e| format!("read sha256sums: {e}"))?;
    if sums.trim().is_empty() {
        return Err("sha256sums is empty".to_string());
    }
    for line in sums.lines() {
        let (want, name) = line
            .split_once("  ")
            .ok_or_else(|| format!("malformed sha256sums line: {line:?}"))?;
        if name.contains('/') || name.contains("..") {
            return Err(format!("sha256sums names an unsafe path: {name:?}"));
        }
        let bytes = std::fs::read(backup_dir.join(name))
            .map_err(|e| format!("read {name} for checksums: {e}"))?;
        let got = handbook_store::store::sha256_hex(&bytes);
        if got != want {
            return Err(format!(
                "sha256 mismatch on {name} (backup tampered or torn)"
            ));
        }
    }
    Ok(())
}

/// Extract a backup tarball into `data` after rejecting unsafe
/// members (absolute paths, `..` components — any bad line fails
/// the whole extract, so names split across lines fail closed too).
/// Afterwards the tree must hold no symlinks and every path must
/// resolve inside `data` (a violation deletes the extracted tree
/// and fails loudly).
fn extract_files(archive: &Path, data: &Path) -> Result<(), String> {
    let list = std::process::Command::new("tar")
        .arg("-tzf")
        .arg(archive)
        .output()
        .map_err(|e| format!("list tar: {e}"))?;
    if !list.status.success() {
        return Err(format!(
            "tar list: {}",
            String::from_utf8_lossy(&list.stderr).trim()
        ));
    }
    reject_unsafe_members(&String::from_utf8_lossy(&list.stdout))?;
    let out = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(data)
        .output()
        .map_err(|e| format!("extract tar: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "tar extract: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    if let Err(e) = check_extracted_tree(data) {
        let _ = std::fs::remove_dir_all(data);
        return Err(e);
    }
    Ok(())
}

/// Reject a `tar -tzf` listing holding an absolute member or a `..`
/// component. Legit backups only ever hold relative store-subdir
/// members, so anything else fails closed.
fn reject_unsafe_members(listing: &str) -> Result<(), String> {
    for member in listing.lines() {
        let path = Path::new(member);
        let unsafe_member = path.is_absolute()
            || path
                .components()
                .any(|c| c == std::path::Component::ParentDir);
        if unsafe_member {
            return Err(format!("tar member escapes the restore dir: {member:?}"));
        }
    }
    Ok(())
}

/// Walk an extracted tree: no symlinks (legit backups never hold
/// any — the store creates plain files and dirs only) and every
/// path canonicalizes inside `data`.
fn check_extracted_tree(data: &Path) -> Result<(), String> {
    let canonical = data
        .canonicalize()
        .map_err(|e| format!("canonicalize {}: {e}", data.display()))?;
    let mut stack = vec![data.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            std::fs::read_dir(&dir).map_err(|e| format!("list {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("read entry: {e}"))?;
            let path = entry.path();
            if entry
                .file_type()
                .map_err(|e| format!("file type {}: {e}", path.display()))?
                .is_symlink()
            {
                return Err(format!("extracted symlink refused: {}", path.display()));
            }
            let resolved = path
                .canonicalize()
                .map_err(|e| format!("canonicalize {}: {e}", path.display()))?;
            if !resolved.starts_with(&canonical) {
                return Err(format!(
                    "extracted path escapes scratch: {}",
                    path.display()
                ));
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    Ok(())
}

/// UTC civil date for a unix timestamp (Howard Hinnant's
/// days-to-civil algorithm; no date crate needed for one format).
fn civil_date(unix_secs: i64) -> (i32, u32, u32) {
    let days = unix_secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y } as i32, m, d)
}

/// True when `unix_secs` falls on a Sunday UTC (1970-01-01 was a
/// Thursday, so Sunday is `days % 7 == 3`).
fn is_sunday(unix_secs: i64) -> bool {
    unix_secs.div_euclid(86_400).rem_euclid(7) == 3
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thu 2026-10-29, Fri 2026-10-30, Sun 2026-11-01, noon UTC.
    const THURSDAY: i64 = 1793275200;
    const FRIDAY: i64 = 1793361600;
    const SUNDAY: i64 = 1793534400;

    /// Migrated store (both DBs) with one blob + one text file.
    /// Returns the tempdir (kept alive), the store, and the blob sha.
    fn seed() -> (tempfile::TempDir, handbook_store::store::Store, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = handbook_store::store::Store::new(dir.path().join("store"));
        store.init().unwrap();
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        let p2 = handbook_store::db::open_p2(&store).unwrap();
        handbook_store::db::migrate_p2(&p2).unwrap();
        let sha = store.write_blob(b"backup-me").unwrap();
        std::fs::write(store.root().join("text").join("9.txt"), "nine").unwrap();
        (dir, store, sha)
    }

    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn child_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn civil_date_and_sunday_known_values() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(3 * 86_400), (1970, 1, 4));
        assert_eq!(civil_date(THURSDAY), (2026, 10, 29));
        assert_eq!(civil_date(FRIDAY), (2026, 10, 30));
        assert_eq!(civil_date(SUNDAY), (2026, 11, 1));
        assert!(is_sunday(3 * 86_400));
        assert!(is_sunday(SUNDAY));
        assert!(!is_sunday(0));
        assert!(!is_sunday(THURSDAY));
    }

    #[test]
    fn round_trip_verify_green_and_live_untouched() {
        let (dir, store, blob_sha) = seed();
        let live_main = std::fs::read(store.db_path()).unwrap();
        let live_p2 = std::fs::read(store.p2_db_path()).unwrap();
        let live_children = child_names(store.root());
        let dest = dir.path().join("backups");

        let report = backup_store(store.root(), &dest, THURSDAY).unwrap();
        assert_eq!(report.id, "2026-10-29");
        assert_eq!(report.files.len(), 3);
        let daily = dest.join("daily").join("2026-10-29");

        // Exact file set: no WAL/-shm residue next to the copies.
        assert_eq!(
            child_names(&daily),
            [
                "files.tar.gz",
                "handbook.sqlite",
                "manifest.json",
                "p2.sqlite",
                "sha256sums"
            ]
        );
        // Owner-only: PII + TLS key material inside.
        assert_eq!(mode_of(&daily), 0o700);
        for name in child_names(&daily) {
            assert_eq!(mode_of(&daily.join(&name)), 0o600, "{name}");
        }
        assert!(
            child_names(&dest)
                .iter()
                .all(|n| !n.starts_with(".staging"))
        );

        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(daily.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["backup_date"], "2026-10-29");
        assert_eq!(
            manifest["handbook_user_version"],
            handbook_store::db::SCHEMA_VERSION
        );
        assert_eq!(manifest["p2_user_version"], 2);

        // Backup reads the live store only: bytes + dir entries equal.
        assert_eq!(std::fs::read(store.db_path()).unwrap(), live_main);
        assert_eq!(std::fs::read(store.p2_db_path()).unwrap(), live_p2);
        assert_eq!(child_names(store.root()), live_children);

        let scratch = dir.path().join("scratch");
        assert_eq!(verify_backup(&daily, &scratch).unwrap(), "2026-10-29");
        let restored = handbook_store::store::Store::new(scratch.join("data"));
        assert_eq!(restored.read_blob(&blob_sha).unwrap(), b"backup-me");
        assert_eq!(
            std::fs::read(scratch.join("data").join("text").join("9.txt")).unwrap(),
            b"nine"
        );
    }

    #[test]
    fn corrupt_db_refused_and_prior_daily_intact() {
        let (dir, store, _) = seed();
        let dest = dir.path().join("backups");
        backup_store(store.root(), &dest, THURSDAY).unwrap();

        std::fs::write(store.db_path(), b"not a database").unwrap();
        let Err(e) = backup_store(store.root(), &dest, FRIDAY) else {
            panic!("corrupt live db was kept as a backup");
        };
        assert!(e.contains("handbook.sqlite"), "{e}");

        // No Friday daily, no leftover staging, Thursday still green.
        assert!(!dest.join("daily").join("2026-10-30").exists());
        assert!(
            child_names(&dest)
                .iter()
                .all(|n| !n.starts_with(".staging"))
        );
        verify_backup(
            &dest.join("daily").join("2026-10-29"),
            &dir.path().join("scratch"),
        )
        .unwrap();
    }

    #[test]
    fn missing_p2_db_refused() {
        let (dir, store, _) = seed();
        std::fs::remove_file(store.p2_db_path()).unwrap();
        let Err(e) = backup_store(store.root(), &dir.path().join("backups"), THURSDAY) else {
            panic!("partial store was kept as a backup");
        };
        assert!(e.contains("p2.sqlite"), "{e}");
    }

    #[test]
    fn same_day_rerun_refused() {
        let (dir, store, _) = seed();
        let dest = dir.path().join("backups");
        backup_store(store.root(), &dest, THURSDAY).unwrap();
        let Err(e) = backup_store(store.root(), &dest, THURSDAY) else {
            panic!("same-day rerun overwrote a published daily");
        };
        assert!(e.contains("already exists"), "{e}");
    }

    #[test]
    fn verify_rejects_tampered_tar() {
        let (dir, store, _) = seed();
        let dest = dir.path().join("backups");
        backup_store(store.root(), &dest, THURSDAY).unwrap();
        let daily = dest.join("daily").join("2026-10-29");
        let mut bytes = std::fs::read(daily.join("files.tar.gz")).unwrap();
        bytes[100] ^= 0xff;
        std::fs::write(daily.join("files.tar.gz"), &bytes).unwrap();

        let Err(e) = verify_backup(&daily, &dir.path().join("scratch")) else {
            panic!("tampered tarball verified green");
        };
        assert!(e.contains("sha256 mismatch"), "{e}");
    }

    #[test]
    fn verify_refuses_nonempty_scratch() {
        let (dir, store, _) = seed();
        let dest = dir.path().join("backups");
        backup_store(store.root(), &dest, THURSDAY).unwrap();
        let scratch = dir.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("mine.txt"), "do not touch").unwrap();

        let Err(e) = verify_backup(&dest.join("daily").join("2026-10-29"), &scratch) else {
            panic!("non-empty scratch was restored into");
        };
        assert!(e.contains("not empty"), "{e}");
        assert!(scratch.join("mine.txt").is_file());
    }

    #[test]
    fn reject_unsafe_members_cases() {
        assert!(reject_unsafe_members("blob/xx/yy/zz\ntext/9.txt\n").is_ok());
        assert!(reject_unsafe_members("").is_ok());
        assert!(reject_unsafe_members("../evil").is_err());
        assert!(reject_unsafe_members("blob/../../evil").is_err());
        assert!(reject_unsafe_members("/abs/path").is_err());
        assert!(reject_unsafe_members("ok.txt\n../evil\n").is_err());
    }

    #[test]
    fn extracted_tree_refuses_symlink() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), "x").unwrap();
        std::os::unix::fs::symlink("f", dir.path().join("link")).unwrap();
        let Err(e) = check_extracted_tree(dir.path()) else {
            panic!("symlink in extracted tree accepted");
        };
        assert!(e.contains("symlink"), "{e}");
    }

    #[test]
    fn prune_keeps_newest_seven_and_ignores_strays() {
        let dir = tempfile::tempdir().unwrap();
        let daily = dir.path().join("daily");
        for day in 1..=10 {
            std::fs::create_dir_all(daily.join(format!("2026-10-{day:02}"))).unwrap();
        }
        std::fs::write(daily.join("notes.txt"), "keep").unwrap();
        std::fs::create_dir_all(daily.join("tmpwork")).unwrap();

        let victims = prune_backups(&daily, KEEP_DAILY).unwrap();
        assert_eq!(victims, ["2026-10-01", "2026-10-02", "2026-10-03"]);
        assert!(daily.join("2026-10-10").is_dir());
        assert!(daily.join("notes.txt").is_file());
        assert!(daily.join("tmpwork").is_dir());
        assert_eq!(
            child_names(&daily)
                .iter()
                .filter(|n| is_backup_id(n))
                .count(),
            7
        );
    }

    #[test]
    fn prune_never_empties() {
        let dir = tempfile::tempdir().unwrap();
        let weekly = dir.path().join("weekly");
        std::fs::create_dir_all(weekly.join("2026-11-01")).unwrap();
        assert!(prune_backups(&weekly, KEEP_WEEKLY).unwrap().is_empty());
        assert!(weekly.join("2026-11-01").is_dir());
    }

    #[test]
    fn sunday_promotes_weekly_weekday_skips() {
        let (dir, store, _) = seed();
        let dest = dir.path().join("backups");

        let report = backup_store(store.root(), &dest, THURSDAY).unwrap();
        assert!(report.promoted_weekly.is_none());
        assert!(!dest.join("weekly").exists());

        let report = backup_store(store.root(), &dest, SUNDAY).unwrap();
        assert_eq!(report.promoted_weekly.as_deref(), Some("2026-11-01"));
        let weekly = dest.join("weekly").join("2026-11-01");
        assert!(weekly.join("manifest.json").is_file());
        verify_backup(&weekly, &dir.path().join("scratch")).unwrap();
    }

    #[test]
    fn young_staging_left_alone() {
        let (dir, store, _) = seed();
        let dest = dir.path().join("backups");
        std::fs::create_dir_all(&dest).unwrap();
        let other = dest.join(".staging.999.aaa");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("marker"), "live run").unwrap();

        backup_store(store.root(), &dest, THURSDAY).unwrap();
        assert_eq!(
            std::fs::read_to_string(other.join("marker")).unwrap(),
            "live run"
        );
    }

    /// A8 regression guard: the backup unit must keep its hardening
    /// (owner-only perms, no network) across edits.
    #[test]
    fn backup_unit_keeps_hardening() {
        let unit = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../docs/ops/apostille-backup.service"
        ))
        .unwrap();
        for line in [
            "UMask=0077",
            "StateDirectoryMode=0700",
            "RestrictAddressFamilies=AF_UNIX",
            "--store /var/lib/apostille",
            "--dest /var/lib/apostille-backups",
        ] {
            assert!(unit.contains(line), "backup unit lost: {line}");
        }
    }
}
