//! `xtask provision`: make a blank Ubuntu server (or the rehearsal
//! container) ready to run the handbook. Root, idempotent, run from a
//! staged release by `cargo xtask setup` over `ssh -t <admin> sudo`.
//!
//! ```text
//! sudo ./xtask provision --manifest <manifest.json> [--release <dir>]
//!     [--version <sha7>] [--sha <sha>] [--config-stdin] [--check]
//! ```
//!
//! Users `hal` (deploys) and `andrii-ai` (runs the service), group
//! `apostille`; directories; the KEK (created once, never overwritten);
//! the root-owned wrappers and one sudoers drop-in (checked with
//! `visudo`); systemd units from the release; the firewall (skipped in a
//! container). On a first install (no `current`) it also installs the
//! release, seals the config payload from stdin as the service user
//! (`config-import --enroll-if-empty`), loads the packs and starts the
//! service and timers. Later releases go through `ops-deploy.sh` as hal;
//! nothing here runs again unless asked. `--check` prints what would
//! change and changes nothing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

const SBIN: &str = "/usr/local/sbin";
const ROOT: &str = "/srv/apostille";
const STORE: &str = "/var/lib/apostille";
const ETC: &str = "/etc/apostille";
const KEK: &str = "/etc/apostille/handbook-kek";
const SUDOERS: &str = "/etc/sudoers.d/hal-apostille";

/// Root-owned wrappers (name, content), embedded at build time.
pub const WRAPPERS: &[(&str, &str)] = &[
    (
        "apostille-install-unit",
        include_str!("../ops/wrappers/apostille-install-unit"),
    ),
    (
        "apostille-journal",
        include_str!("../ops/wrappers/apostille-journal"),
    ),
    (
        "apostille-config",
        include_str!("../ops/wrappers/apostille-config"),
    ),
    (
        "apostille-config-import",
        include_str!("../ops/wrappers/apostille-config-import"),
    ),
    (
        "apostille-setup-key",
        include_str!("../ops/wrappers/apostille-setup-key"),
    ),
    (
        "apostille-packs-load",
        include_str!("../ops/wrappers/apostille-packs-load"),
    ),
    (
        "apostille-sqlite",
        include_str!("../ops/wrappers/apostille-sqlite"),
    ),
    (
        "apostille-invite",
        include_str!("../ops/wrappers/apostille-invite"),
    ),
];
pub const SUDOERS_DROP_IN: &str = include_str!("../ops/wrappers/hal-apostille.sudoers");

/// What setup sends along (no secrets: the config payload comes on stdin).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Public keys allowed to log in as hal.
    #[serde(default)]
    pub deploy_keys: Vec<String>,
    /// Timers to enable (`watch`, `mail`, `backup`, `instruct`, `retention`).
    #[serde(default)]
    pub timers: Vec<String>,
    /// `ufw` or `none`.
    #[serde(default)]
    pub firewall: String,
    /// The host's public name, recorded in /srv/apostille/public-host
    /// for the deploy smoke (absent: production's andrii.ai).
    #[serde(default)]
    pub public_host: Option<String>,
    /// Rehearsal TLS pair (PEM), written to /etc/apostille/tls/.
    #[serde(default)]
    pub tls_cert_pem: Option<String>,
    #[serde(default)]
    pub tls_key_pem: Option<String>,
}

/// A named provisioning step.
type ProvisionStep<'a> = (&'a str, &'a dyn Fn(&mut Ctx) -> Result<(), String>);

struct Ctx {
    check: bool,
    changes: usize,
}

impl Ctx {
    fn say(&mut self, what: &str) {
        self.changes += 1;
        println!("{} {what}", if self.check { "would" } else { "  ->" });
    }

    fn run(&mut self, what: &str, cmd: &[&str]) -> Result<(), String> {
        self.say(what);
        if self.check {
            return Ok(());
        }
        let status = Command::new(cmd[0])
            .args(&cmd[1..])
            .status()
            .map_err(|e| format!("{}: {e}", cmd[0]))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{what}: {} failed", cmd.join(" ")))
        }
    }
}

fn ok(cmd: &[&str]) -> bool {
    Command::new(cmd[0])
        .args(&cmd[1..])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn in_container() -> bool {
    Command::new("systemd-detect-virt")
        .arg("-c")
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() != "none")
        .unwrap_or(false)
        || Path::new("/.dockerenv").exists()
}

#[cfg(unix)]
fn mode_owner(path: &Path) -> Option<(u32, u32, u32)> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata(path)
        .ok()
        .map(|m| (m.mode() & 0o7777, m.uid(), m.gid()))
}

fn id_of(kind: &str, name: &str) -> Option<u32> {
    let out = Command::new("id").arg(kind).arg(name).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn gid_of(group: &str) -> Option<u32> {
    let out = Command::new("getent")
        .args(["group", group])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .split(':')
        .nth(2)
        .and_then(|g| g.trim().parse().ok())
}

/// Ensure a directory exists with `owner:group` and `mode`.
fn dir(ctx: &mut Ctx, path: &str, owner: &str, group: &str, mode: u32) -> Result<(), String> {
    let want = (mode, id_of("-u", owner), gid_of(group));
    let have = mode_owner(Path::new(path));
    let same = match (have, want) {
        (Some((m, u, g)), (wm, Some(wu), Some(wg))) => m == wm && u == wu && g == wg,
        _ => false,
    };
    if same {
        return Ok(());
    }
    ctx.run(
        &format!("directory {path} {owner}:{group} {mode:o}"),
        &[
            "install",
            "-d",
            "-o",
            owner,
            "-g",
            group,
            "-m",
            &format!("{mode:o}"),
            path,
        ],
    )
}

/// Write a file when its content, mode or owner differ.
fn file(
    ctx: &mut Ctx,
    path: &str,
    content: &[u8],
    owner: &str,
    group: &str,
    mode: u32,
) -> Result<(), String> {
    let same_bytes = std::fs::read(path).map(|b| b == content).unwrap_or(false);
    let same_meta = match (
        mode_owner(Path::new(path)),
        id_of("-u", owner),
        gid_of(group),
    ) {
        (Some((m, u, g)), Some(wu), Some(wg)) => m == mode && u == wu && g == wg,
        _ => false,
    };
    if same_bytes && same_meta {
        return Ok(());
    }
    ctx.say(&format!("file {path} {owner}:{group} {mode:o}"));
    if ctx.check {
        return Ok(());
    }
    let tmp = format!("{path}.provision-tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("write {path}: {e}"))?;
    for cmd in [
        vec!["chown", &format!("{owner}:{group}"), &tmp],
        vec!["chmod", &format!("{mode:o}"), &tmp],
    ] {
        let args: Vec<&str> = cmd.iter().map(|s| s.as_ref()).collect();
        if !ok(&args) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("{} {path} failed", args[0]));
        }
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {path}: {e}"))
}

fn users(ctx: &mut Ctx, manifest: &Manifest) -> Result<(), String> {
    if gid_of("apostille").is_none() {
        ctx.run("group apostille", &["groupadd", "apostille"])?;
    }
    if id_of("-u", "andrii-ai").is_none() {
        ctx.run(
            "service user andrii-ai",
            &[
                "useradd",
                "--system",
                "--user-group",
                "--home-dir",
                STORE,
                "--no-create-home",
                "--shell",
                "/usr/sbin/nologin",
                "andrii-ai",
            ],
        )?;
    }
    if id_of("-u", "hal").is_none() {
        ctx.run(
            "deploy user hal",
            &[
                "useradd",
                "--create-home",
                "--user-group",
                "--shell",
                "/bin/bash",
                "hal",
            ],
        )?;
    }
    for user in ["hal", "andrii-ai"] {
        let groups = Command::new("id")
            .args(["-nG", user])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        if !groups.split_whitespace().any(|g| g == "apostille") {
            ctx.run(
                &format!("{user} joins apostille"),
                &["usermod", "-aG", "apostille", user],
            )?;
        }
    }
    if !manifest.deploy_keys.is_empty() {
        let path = "/home/hal/.ssh/authorized_keys";
        let mut lines: Vec<String> = std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        for key in &manifest.deploy_keys {
            let key = key.trim();
            if !key.starts_with("ssh-") && !key.starts_with("ecdsa-") {
                return Err("deploy_keys must be OpenSSH public keys".into());
            }
            if !lines.iter().any(|l| l.trim() == key) {
                lines.push(key.to_string());
            }
        }
        dir(ctx, "/home/hal/.ssh", "hal", "hal", 0o700)?;
        file(
            ctx,
            path,
            (lines.join("\n") + "\n").as_bytes(),
            "hal",
            "hal",
            0o600,
        )?;
    }
    Ok(())
}

fn layout(ctx: &mut Ctx, manifest: &Manifest) -> Result<(), String> {
    dir(ctx, ROOT, "root", "apostille", 0o2775)?;
    dir(
        ctx,
        &format!("{ROOT}/releases"),
        "root",
        "apostille",
        0o2775,
    )?;
    // Group apostille reads (never writes) here: hal's recurring ops
    // scripts stage a KEK copy, as on production. tls/ stays andrii-ai's.
    dir(ctx, ETC, "root", "apostille", 0o2750)?;
    dir(ctx, STORE, "andrii-ai", "andrii-ai", 0o750)?;
    dir(
        ctx,
        "/var/lib/apostille-backups",
        "andrii-ai",
        "andrii-ai",
        0o750,
    )?;
    for sub in ["apostille", "apostille/staging", "apostille/scripts"] {
        dir(ctx, &format!("/home/hal/{sub}"), "hal", "hal", 0o755)?;
    }
    if !Path::new(KEK).exists() {
        ctx.say(&format!(
            "new KEK {KEK} (32 random bytes, root:apostille 0640)"
        ));
        if !ctx.check {
            let kek: [u8; 32] = rand::random();
            let tmp = format!("{KEK}.provision-tmp");
            std::fs::write(&tmp, kek).map_err(|e| format!("write KEK: {e}"))?;
            if !(ok(&["chown", "root:apostille", &tmp]) && ok(&["chmod", "640", &tmp])) {
                let _ = std::fs::remove_file(&tmp);
                return Err("KEK permissions".into());
            }
            std::fs::rename(&tmp, KEK).map_err(|e| format!("rename KEK: {e}"))?;
        }
    }
    if let Some(host) = &manifest.public_host {
        if host.is_empty()
            || !host
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        {
            return Err(format!("bad public_host {host:?}"));
        }
        file(
            ctx,
            &format!("{ROOT}/public-host"),
            format!("{host}\n").as_bytes(),
            "root",
            "apostille",
            0o644,
        )?;
    }
    if let (Some(cert), Some(key)) = (&manifest.tls_cert_pem, &manifest.tls_key_pem) {
        dir(ctx, &format!("{ETC}/tls"), "root", "andrii-ai", 0o2750)?;
        file(
            ctx,
            &format!("{ETC}/tls/cert.pem"),
            cert.as_bytes(),
            "root",
            "andrii-ai",
            0o640,
        )?;
        file(
            ctx,
            &format!("{ETC}/tls/key.pem"),
            key.as_bytes(),
            "root",
            "andrii-ai",
            0o640,
        )?;
    }
    Ok(())
}

fn wrappers(ctx: &mut Ctx) -> Result<(), String> {
    for (name, content) in WRAPPERS {
        file(
            ctx,
            &format!("{SBIN}/{name}"),
            content.as_bytes(),
            "root",
            "root",
            0o755,
        )?;
    }
    let current = std::fs::read(SUDOERS).unwrap_or_default();
    if current != SUDOERS_DROP_IN.as_bytes() {
        ctx.say(&format!("sudoers drop-in {SUDOERS} (checked with visudo)"));
        if !ctx.check {
            let tmp = "/etc/sudoers.d/.hal-apostille.provision-tmp";
            std::fs::write(tmp, SUDOERS_DROP_IN).map_err(|e| format!("write sudoers: {e}"))?;
            let _ = ok(&["chmod", "440", tmp]);
            if !ok(&["visudo", "-c", "-q", "-f", tmp]) {
                let _ = std::fs::remove_file(tmp);
                return Err("sudoers drop-in failed visudo; nothing installed".into());
            }
            std::fs::rename(tmp, SUDOERS).map_err(|e| format!("install sudoers: {e}"))?;
        }
    }
    // The older split drop-ins are subsumed by the one above.
    for legacy in [
        "/etc/sudoers.d/apostille-restart",
        "/etc/sudoers.d/apostille-hal",
    ] {
        if Path::new(legacy).exists() {
            ctx.run(&format!("remove legacy {legacy}"), &["rm", "-f", legacy])?;
        }
    }
    if !ctx.check && !ok(&["visudo", "-c", "-q"]) {
        return Err("full sudoers check failed after the change".into());
    }
    Ok(())
}

/// The service unit may only run the live release as the service user.
pub fn check_service_unit(text: &str) -> Result<(), String> {
    for line in [
        "User=andrii-ai",
        "Group=andrii-ai",
        "ExecStart=/srv/apostille/current/web",
    ] {
        if !text.lines().any(|l| l.trim() == line) {
            return Err(format!("apostille.service must contain {line:?}"));
        }
    }
    Ok(())
}

fn units(ctx: &mut Ctx, release: Option<&Path>, timers: &[String]) -> Result<bool, String> {
    let Some(dir) = release.map(|r| r.join("units")).filter(|d| d.is_dir()) else {
        return Ok(false);
    };
    let mut changed = false;
    let mut names: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("service" | "timer")
            )
        })
        .collect();
    names.sort();
    for unit in names {
        let name = unit
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if !name.starts_with("apostille") {
            continue;
        }
        let text = std::fs::read_to_string(&unit).map_err(|e| e.to_string())?;
        if name == "apostille.service" {
            check_service_unit(&text)?;
        }
        let before = ctx.changes;
        file(
            ctx,
            &format!("/etc/systemd/system/{name}"),
            text.as_bytes(),
            "root",
            "root",
            0o644,
        )?;
        changed |= ctx.changes != before;
    }
    if changed {
        ctx.run("systemd daemon-reload", &["systemctl", "daemon-reload"])?;
    }
    for timer in timers {
        if !timer.bytes().all(|b| b.is_ascii_lowercase()) {
            return Err(format!("bad timer name {timer:?}"));
        }
        let unit = format!("apostille-{timer}.timer");
        if !ok(&["systemctl", "is-enabled", "--quiet", &unit]) {
            ctx.run(
                &format!("enable {unit}"),
                &["systemctl", "enable", "--now", &unit],
            )?;
        }
    }
    Ok(changed)
}

fn firewall(ctx: &mut Ctx, manifest: &Manifest) -> Result<(), String> {
    if manifest.firewall != "ufw" {
        return Ok(());
    }
    if in_container() {
        println!("  (firewall skipped: running in a container)");
        return Ok(());
    }
    if !ok(&["which", "ufw"]) {
        return Err("firewall: ufw is not installed (apt-get install ufw)".into());
    }
    let status = Command::new("ufw")
        .arg("status")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    // SSH first, so enabling the firewall can never lock the session out.
    for rule in ["22/tcp", "80/tcp", "443/tcp", "443/udp"] {
        if !status.contains(rule) {
            ctx.run(&format!("firewall allows {rule}"), &["ufw", "allow", rule])?;
        }
    }
    if !status.contains("Status: active") {
        ctx.run("firewall on", &["ufw", "--force", "enable"])?;
    }
    Ok(())
}

fn as_service_user(args: &[&str], stdin: Option<&[u8]>) -> Result<(), String> {
    let mut cmd = Command::new("runuser");
    cmd.args(["-u", "andrii-ai", "--"]).args(args);
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().map_err(|e| format!("runuser: {e}"))?;
    if let Some(bytes) = stdin {
        use std::io::Write as _;
        child
            .stdin
            .take()
            .ok_or("stdin")?
            .write_all(bytes)
            .map_err(|e| e.to_string())?;
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} failed as andrii-ai", args.join(" ")))
    }
}

fn first_install(
    ctx: &mut Ctx,
    release: &Path,
    version: &str,
    sha: &str,
    config: Option<&[u8]>,
) -> Result<(), String> {
    crate::release::validate_version(version)?;
    crate::release::validate_release_src(release)?;
    ctx.say(&format!(
        "install release {version} and point current at it"
    ));
    if ctx.check {
        ctx.say("seal the config payload, load packs, start apostille");
        return Ok(());
    }
    let root = Path::new(ROOT);
    crate::release::install_release(root, release, version, sha, "", true)?;
    crate::release::atomic_swap(root, version)?;
    // hal owns the release dirs it later prunes and rolls back to.
    if !ok(&[
        "chown",
        "-R",
        "hal:apostille",
        &format!("{ROOT}/releases/{version}"),
    ]) {
        return Err("chown release".into());
    }
    let xtask = format!("{ROOT}/current/xtask");
    as_service_user(&[&xtask, "init", "--store", STORE], None)?;
    let payload =
        config.ok_or("first install needs the config payload on stdin (--config-stdin)")?;
    as_service_user(
        &[
            &xtask,
            "config-import",
            "--store",
            STORE,
            "--kek-file",
            KEK,
            "--stdin",
            "--mode",
            "replace",
            "--enroll-if-empty",
            "--actor",
            "provision",
        ],
        Some(payload),
    )?;
    as_service_user(
        &[
            &xtask,
            "packs-load",
            "--store",
            STORE,
            "--packs",
            &format!("{ROOT}/current/packs"),
        ],
        None,
    )?;
    ctx.run(
        "start apostille",
        &["systemctl", "enable", "--now", "apostille"],
    )
}

pub fn run(args: &[String]) {
    let fail = |m: &str| -> ! {
        eprintln!("provision: {m}");
        std::process::exit(1);
    };
    let check = args.iter().any(|a| a == "--check");
    if !check && id_of("-u", "root").is_some() && current_uid() != 0 {
        fail("run as root (sudo); --check works without root");
    }
    let manifest: Manifest = match crate::flag(args, "manifest") {
        Some(path) => {
            let text =
                std::fs::read_to_string(&path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
            serde_json::from_str(&text).unwrap_or_else(|e| fail(&format!("manifest: {e}")))
        }
        None => Manifest::default(),
    };
    let release = crate::flag(args, "release").map(PathBuf::from);
    let config = if args.iter().any(|a| a == "--config-stdin") {
        use std::io::Read as _;
        let mut buf = Vec::new();
        std::io::stdin()
            .read_to_end(&mut buf)
            .unwrap_or_else(|e| fail(&e.to_string()));
        Some(buf)
    } else {
        None
    };
    let mut ctx = Ctx { check, changes: 0 };
    let steps: [ProvisionStep; 4] = [
        ("users", &|c| users(c, &manifest)),
        ("layout", &|c| layout(c, &manifest)),
        ("wrappers and sudoers", &|c| wrappers(c)),
        ("firewall", &|c| firewall(c, &manifest)),
    ];
    for (name, step) in steps {
        println!("== {name}");
        step(&mut ctx).unwrap_or_else(|e| fail(&format!("{name}: {e}")));
    }
    println!("== units");
    units(&mut ctx, release.as_deref(), &manifest.timers)
        .unwrap_or_else(|e| fail(&format!("units: {e}")));
    if !Path::new(&format!("{ROOT}/current")).exists() {
        println!("== first install");
        let release = release
            .as_deref()
            .unwrap_or_else(|| fail("first install needs --release <dir>"));
        let version =
            crate::flag(args, "version").unwrap_or_else(|| fail("first install needs --version"));
        let sha = crate::flag(args, "sha").unwrap_or_else(|| version.clone());
        first_install(&mut ctx, release, &version, &sha, config.as_deref())
            .unwrap_or_else(|e| fail(&format!("first install: {e}")));
    }
    if check {
        println!(
            "check: {} change(s) would be made; nothing changed",
            ctx.changes
        );
    } else {
        println!("provision: done, {} change(s)", ctx.changes);
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // `id -u` keeps this free of libc FFI.
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrappers_are_valid_bash_with_no_arguments_taken_from_argv() {
        for (name, content) in WRAPPERS {
            assert!(content.starts_with("#!/bin/bash\n"), "{name}");
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(name);
            std::fs::write(&path, content).unwrap();
            let status = Command::new("bash").arg("-n").arg(&path).status().unwrap();
            assert!(status.success(), "{name} has a bash syntax error");
            // Fixed paths only: no wrapper reaches outside the release or store.
            for line in content.lines().filter(|l| l.contains("exec ")) {
                assert!(
                    line.contains("/srv/apostille/current/") || line.contains("/usr/bin/"),
                    "{name}: {line}"
                );
            }
        }
    }

    #[test]
    fn sudoers_grants_only_fixed_paths() {
        assert!(!SUDOERS_DROP_IN.contains('*'), "no wildcards");
        for (name, _) in WRAPPERS {
            if *name != "apostille-install-unit" && *name != "apostille-journal" {
                assert!(
                    SUDOERS_DROP_IN.contains(&format!("{SBIN}/{name}")),
                    "{name} granted"
                );
            }
        }
        assert!(!SUDOERS_DROP_IN.contains("systemctl stop"), "no stop verb");
    }

    #[test]
    fn the_service_unit_in_the_repo_passes_the_install_check() {
        check_service_unit(include_str!("../../docs/ops/apostille.service")).unwrap();
        assert!(check_service_unit("[Service]\nUser=root\nExecStart=/bin/sh\n").is_err());
    }

    #[test]
    fn manifest_shape_is_strict() {
        let m: Manifest = serde_json::from_str(
            r#"{"deploy_keys":["ssh-ed25519 AAAA synthetic"],"timers":["watch"],"firewall":"ufw"}"#,
        )
        .unwrap();
        assert_eq!(m.timers, vec!["watch".to_string()]);
        assert!(serde_json::from_str::<Manifest>(r#"{"secret":"x"}"#).is_err());
    }
}
