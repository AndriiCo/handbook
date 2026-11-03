//! Run a plan: each step's output streams line by line as [`Event`]s
//! (the terminal prints them; the wizard forwards them over SSE).
//! Stdin feeds (config payload, provision manifest) are rendered here,
//! at run time, and never printed or written to disk.

use std::io::{BufRead as _, BufReader, Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

use super::file::{SetupFile, TargetKind};
use super::plan::{Action, Builtin, Conn, Facts, Feed, Phase, REHEARSAL_DIR, Step};
use super::{build, rehearsal, stage};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Plan {
        target: String,
        version: String,
        steps: Vec<[String; 2]>,
    },
    Start {
        index: usize,
        phase: String,
        title: String,
    },
    Line {
        index: usize,
        text: String,
    },
    Done {
        index: usize,
        ok: bool,
        detail: String,
    },
    Finished {
        target: String,
        ok: bool,
        detail: String,
    },
}

pub struct Ctx<'a> {
    pub repo: PathBuf,
    pub file: &'a SetupFile,
    pub target: String,
    pub facts: Facts,
    pub emit: &'a (dyn Fn(Event) + Sync),
}

impl Ctx<'_> {
    fn line(&self, index: usize, text: impl Into<String>) {
        (self.emit)(Event::Line {
            index,
            text: text.into(),
        });
    }
}

/// Run `steps` in order, from the first step of `from` when given.
pub fn execute(ctx: &Ctx, steps: &[Step], from: Option<Phase>) -> Result<(), String> {
    (ctx.emit)(Event::Plan {
        target: ctx.target.clone(),
        version: ctx.facts.version().to_string(),
        steps: steps
            .iter()
            .map(|s| [s.phase.as_str().to_string(), s.title.clone()])
            .collect(),
    });
    let start = match from {
        Some(phase) => steps
            .iter()
            .position(|s| s.phase == phase)
            .ok_or_else(|| format!("this plan has no {} phase", phase.as_str()))?,
        None => 0,
    };
    for (index, s) in steps.iter().enumerate().skip(start) {
        (ctx.emit)(Event::Start {
            index,
            phase: s.phase.as_str().to_string(),
            title: s.title.clone(),
        });
        let result = run_action(ctx, index, &s.action);
        let (ok, detail) = match &result {
            Ok(()) => (true, String::new()),
            Err(e) => (false, e.clone()),
        };
        (ctx.emit)(Event::Done { index, ok, detail });
        if let Err(e) = result {
            let detail = format!(
                "step {} [{}] failed: {e} (resume with --from {})",
                index + 1,
                s.phase.as_str(),
                s.phase.as_str()
            );
            (ctx.emit)(Event::Finished {
                target: ctx.target.clone(),
                ok: false,
                detail: detail.clone(),
            });
            return Err(detail);
        }
    }
    (ctx.emit)(Event::Finished {
        target: ctx.target.clone(),
        ok: true,
        detail: format!("{} is up on {}", ctx.target, ctx.facts.version()),
    });
    Ok(())
}

fn run_action(ctx: &Ctx, index: usize, action: &Action) -> Result<(), String> {
    match action {
        Action::Local { argv } => {
            let mut cmd = Command::new(&argv[0]);
            cmd.args(&argv[1..])
                .current_dir(&ctx.repo)
                .stdin(Stdio::null());
            stream(ctx, index, &mut cmd, None)
        }
        Action::Ssh { conn, script, feed } => {
            let input = render_feed(ctx, *feed)?;
            let mut cmd = ssh_command(conn);
            cmd.arg(script).stdin(Stdio::null());
            stream(ctx, index, &mut cmd, input)
        }
        Action::Upload { conn, from, to } => upload(ctx, index, conn, &ctx.repo.join(from), to),
        Action::Builtin(b) => builtin(ctx, index, b),
    }
}

/// `ssh` from the repo root (the rehearsal's key paths are repo-relative).
fn ssh_command(conn: &Conn) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.args(conn.options("-p"))
        .arg(&conn.dest)
        .arg("--")
        .current_dir(crate::repo_root());
    cmd
}

/// Run `cmd`, emitting each stdout/stderr line; feed `input` on stdin.
fn stream(
    ctx: &Ctx,
    index: usize,
    cmd: &mut Command,
    input: Option<Vec<u8>>,
) -> Result<(), String> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    // Without a feed, keep whatever stdin the caller set (an upload's tar
    // pipe); callers default to null.
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
    if let Some(bytes) = input {
        let mut stdin = child.stdin.take().ok_or("stdin")?;
        stdin.write_all(&bytes).map_err(|e| format!("stdin: {e}"))?;
    }
    let out = child.stdout.take().ok_or("stdout")?;
    let err = child.stderr.take().ok_or("stderr")?;
    std::thread::scope(|scope| {
        scope.spawn(|| forward(ctx, index, out));
        scope.spawn(|| forward(ctx, index, err));
    });
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("exit {}", status.code().unwrap_or(-1)))
    }
}

fn forward(ctx: &Ctx, index: usize, pipe: impl Read) {
    for line in BufReader::new(pipe).lines().map_while(Result::ok) {
        ctx.line(index, line);
    }
}

/// `tar` the local directory into a fresh remote one over SSH.
fn upload(ctx: &Ctx, index: usize, conn: &Conn, from: &Path, to: &str) -> Result<(), String> {
    if !from.is_dir() {
        return Err(format!("{} is not a directory", from.display()));
    }
    let bsd = Command::new("tar")
        .arg("--version")
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("bsdtar"));
    let mut tar = Command::new("tar");
    tar.env("COPYFILE_DISABLE", "1");
    if bsd {
        tar.args(["--no-xattrs", "--no-mac-metadata"]);
    }
    let mut tar = tar
        .arg("-C")
        .arg(from)
        .args(["-cf", "-", "."])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("tar: {e}"))?;
    let tar_out = tar.stdout.take().ok_or("tar stdout")?;
    let mut ssh = ssh_command(conn);
    ssh.arg(format!(
        "rm -rf {to} && mkdir -p {to} && tar -C {to} -xf - --no-same-owner && echo uploaded to {to}"
    ))
    .stdin(Stdio::from(tar_out));
    let result = stream(ctx, index, &mut ssh, None);
    let tar_ok = tar.wait().is_ok_and(|s| s.success());
    result?;
    if tar_ok {
        Ok(())
    } else {
        Err("tar failed".into())
    }
}

fn render_feed(ctx: &Ctx, feed: Feed) -> Result<Option<Vec<u8>>, String> {
    match feed {
        Feed::Nothing => Ok(None),
        Feed::ConfigPayload => {
            let payload = ctx.file.import_payload(&ctx.target)?;
            serde_json::to_vec(&payload)
                .map(Some)
                .map_err(|e| e.to_string())
        }
        Feed::Manifest => manifest(ctx).map(Some),
    }
}

/// Provision manifest: deploy keys, timers, firewall, public host and
/// (rehearsal only) the TLS pair.
fn manifest(ctx: &Ctx) -> Result<Vec<u8>, String> {
    let target = &ctx.file.targets[&ctx.target];
    let mut keys = target.deploy_keys.clone();
    let mut tls = (None, None);
    if target.kind == TargetKind::Docker {
        keys.push(rehearsal::ensure_keys(&ctx.repo)?);
        let dir = ctx.repo.join(REHEARSAL_DIR);
        let read = |n: &str| std::fs::read_to_string(dir.join(n)).map_err(|e| format!("{n}: {e}"));
        tls = (Some(read("cert.pem")?), Some(read("key.pem")?));
    }
    if keys.is_empty() {
        // The operator's agent keys, so hal can be reached the same way.
        let out = Command::new("ssh-add")
            .arg("-L")
            .output()
            .map_err(|e| e.to_string())?;
        keys = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.starts_with("ssh-"))
            .map(str::to_string)
            .collect();
    }
    if keys.is_empty() {
        return Err(
            "no deploy key for hal: set targets.<name>.deploy_keys or load a key in ssh-agent"
                .into(),
        );
    }
    let value = serde_json::json!({
        "deploy_keys": keys,
        "timers": target.timers,
        "firewall": if target.kind == TargetKind::Server { "ufw" } else { "none" },
        "public_host": target.public_host,
        "tls_cert_pem": tls.0,
        "tls_key_pem": tls.1,
    });
    serde_json::to_vec(&value).map_err(|e| e.to_string())
}

fn have_tool(tool: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {tool}")])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn builtin(ctx: &Ctx, index: usize, b: &Builtin) -> Result<(), String> {
    let log = |text: String| ctx.line(index, text);
    match b {
        Builtin::Doctor { tools } => {
            let missing: Vec<&str> = tools.iter().copied().filter(|t| !have_tool(t)).collect();
            if !missing.is_empty() {
                return Err(format!("missing tools: {}", missing.join(", ")));
            }
            if tools.contains(&"docker") {
                build::detect_docker()?;
            }
            if tools.contains(&"zig") {
                let v = Command::new("zig")
                    .arg("version")
                    .output()
                    .map_err(|e| e.to_string())?;
                let v = String::from_utf8_lossy(&v.stdout).trim().to_string();
                if v != "0.17.0" {
                    return Err(format!("zig {v} (need 0.17.0)"));
                }
            }
            log(format!("ok: {}", tools.join(", ")));
            Ok(())
        }
        Builtin::BuildRelease { arch } => {
            let brand = super::brand::licensed_files(&ctx.repo, ctx.file);
            stage::stage_release(&ctx.repo, &ctx.facts.sha, *arch, &brand, &log).map(|_| ())
        }
        Builtin::GateStamp => {
            let sha12 = &ctx.facts.sha[..12];
            let mut cmd = ssh_command(&Conn::alias("plant"));
            cmd.stdin(Stdio::null()).arg(format!(
                "grep -l '\"ci_green\": true' ~/apostille-pipeline/acceptance/{sha12}*.json"
            ));
            stream(ctx, index, &mut cmd, None).map_err(|_| {
                format!(
                    "no plant stamp with ci_green for {sha12} (ops/plant-pipeline/submit.sh first)"
                )
            })
        }
        Builtin::DnsCheck { names, bind, bind6 } => {
            dns_check(names, bind.as_deref(), bind6.as_deref(), &log)
        }
        Builtin::RehearsalKeys => {
            rehearsal::ensure_keys(&ctx.repo).map(|_| log("key ready".into()))
        }
        Builtin::RehearsalTls { names } => rehearsal::ensure_tls(&ctx.repo, names)
            .map(|_| log(format!("leaf for {}", names.join(", ")))),
        Builtin::StartContainer {
            platform,
            https,
            http,
        } => {
            let target = &ctx.file.targets[&ctx.target];
            let host = target.public_host.clone().unwrap_or_default();
            let names = if target.dns.is_empty() {
                vec![host.clone(), format!("admin.{host}")]
            } else {
                target.dns.clone()
            };
            rehearsal::start_container(&ctx.repo, platform, (*https, *http), &names, &log)
        }
        Builtin::WaitSsh { conn } => rehearsal::wait_ssh(conn, 90),
        Builtin::WaitHealth {
            url,
            ca,
            timeout_secs,
        } => wait_health(ctx, url, ca.as_deref(), *timeout_secs, &log),
        Builtin::LocalBoot => {
            let target = &ctx.file.targets[&ctx.target];
            let values = ctx.file.effective_values(&ctx.target, target);
            let payload = serde_json::json!({
                "format": crate::config_import::FORMAT,
                "values": values,
            });
            crate::up::set_import_payload(serde_json::to_vec(&payload).map_err(|e| e.to_string())?);
            crate::up::run(&[]);
            Ok(())
        }
    }
}

fn dns_check(
    names: &[String],
    bind: Option<&str>,
    bind6: Option<&str>,
    log: &dyn Fn(String),
) -> Result<(), String> {
    use std::net::ToSocketAddrs as _;
    let mut problems = Vec::new();
    for name in names {
        let addrs: Vec<String> = match (name.as_str(), 443).to_socket_addrs() {
            Ok(a) => a.map(|a| a.ip().to_string()).collect(),
            Err(e) => {
                problems.push(format!("{name}: {e}"));
                continue;
            }
        };
        log(format!("{name} -> {}", addrs.join(" ")));
        for want in [bind, bind6].into_iter().flatten() {
            if !addrs.iter().any(|a| a == want) {
                problems.push(format!("{name} does not resolve to {want}"));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

fn wait_health(
    ctx: &Ctx,
    url: &str,
    ca: Option<&str>,
    timeout: u64,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let want = format!("\"version\":\"0.1.0+{}\"", ctx.facts.version());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
    let mut last = String::new();
    loop {
        let mut cmd = Command::new("curl");
        cmd.args(["-sS", "--max-time", "5"]);
        if let Some(ca) = ca {
            cmd.arg("--cacert").arg(ctx.repo.join(ca));
        }
        let out = cmd.arg(url).output().map_err(|e| format!("curl: {e}"))?;
        let body: String = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .collect();
        if out.status.success() && body.contains("\"status\":\"ok\"") && body.contains(&want) {
            log(body);
            return Ok(());
        }
        let now = if out.status.success() {
            body
        } else {
            String::from_utf8_lossy(&out.stderr).trim().to_string()
        };
        if now != last {
            log(now.clone());
            last = now;
        }
        if std::time::Instant::now() > deadline {
            return Err(format!(
                "{url} never named 0.1.0+{} (last: {last})",
                ctx.facts.version()
            ));
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}
