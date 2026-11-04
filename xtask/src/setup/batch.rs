//! The wizard's "everything at once" controls: Run all, Stop all and
//! Remove all, each a plain plan per target streamed like any run.
//!
//! **Run all** brings up the system map's nodes in release order:
//! staging, then the front end, then the workers (each target once; a
//! native local boot last, since it keeps serving). Production still
//! needs the operator's order, as it does alone.
//!
//! **Stop all** and **Remove all** cover every target in the file that
//! runs on this machine or on plant. A server target is never stopped
//! or removed from here: hal has no stop verb on purpose, and a release
//! goes back with the rollback tools, not by deleting it.
//!
//! - local, native: stop what listens on its two ports from this
//!   checkout; remove empties `.local/` like `up --fresh` (the setup
//!   file stays).
//! - local, Docker: `docker compose stop`; remove is `down --volumes`.
//! - docker rehearsal: stop the container; remove deletes it (its SSH
//!   key and CA under `private/rehearsal/` stay).
//! - plant staging: stop its `web` by verified PID; remove deletes the
//!   staging store and KEK (`docs/ops/deploy.md` § Plant: staging data
//!   may be reset at any time). The checkout stays.

use super::file::{Runtime, SetupFile, Target, TargetKind};
use super::map;
use super::plan::{self, Action, Conn, Feed, Phase, REHEARSAL_CONTAINER, REHEARSAL_DIR, Step};

/// Remove all on a native local target: what `up --fresh` does, so a
/// new worker or cache file never gets left behind. Everything under
/// `.local/` goes except the plain setup file.
const EMPTY_LOCAL: &str = "[ ! -d .local ] || find .local -mindepth 1 -maxdepth 1 ! -name setup.yaml -exec rm -rf {} + ; echo removed";

/// One of the three controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum All {
    Run,
    Stop,
    Remove,
}

impl All {
    pub fn parse(text: &str) -> Option<All> {
        match text {
            "run" => Some(All::Run),
            "stop" => Some(All::Stop),
            "remove" => Some(All::Remove),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            All::Run => "run",
            All::Stop => "stop",
            All::Remove => "remove",
        }
    }
}

fn native(target: &Target) -> bool {
    target.kind == TargetKind::Local && target.runtime == Runtime::Native
}

/// Run all: the map's staging, front end and workers targets, each
/// once, with a native local boot moved last.
pub fn run_order(file: &SetupFile) -> Vec<String> {
    let nodes = map::resolve(file);
    let mut order: Vec<String> = Vec::new();
    for node in ["staging", "front_end", "workers"] {
        if let Some(Some(name)) = nodes.get(node)
            && !order.contains(name)
        {
            order.push(name.clone());
        }
    }
    order.sort_by_key(|name| native(&file.targets[name]));
    order
}

/// Stop all and Remove all: every target that is not a server.
pub fn local_targets(file: &SetupFile) -> Vec<String> {
    file.targets
        .iter()
        .filter(|(_, t)| t.kind != TargetKind::Server)
        .map(|(name, _)| name.clone())
        .collect()
}

/// The server targets Stop all and Remove all leave alone.
pub fn servers(file: &SetupFile) -> Vec<String> {
    file.targets
        .iter()
        .filter(|(_, t)| t.kind == TargetKind::Server)
        .map(|(name, _)| name.clone())
        .collect()
}

fn local(argv: &str) -> Action {
    Action::Local {
        argv: vec!["sh".into(), "-c".into(), argv.into()],
    }
}

/// Kill what listens on `ports` from this checkout (the cwd of the
/// step), and nothing else on those ports.
fn stop_ports(ports: [u16; 2]) -> String {
    let [p, a] = ports;
    format!(
        "here=$(pwd -P); found=0; \
         for port in {p} {a}; do for pid in $(lsof -nP -t -iTCP:$port -sTCP:LISTEN 2>/dev/null); do \
           cwd=$(lsof -a -p $pid -d cwd -Fn 2>/dev/null | sed -n 's/^n//p'); \
           case \"$cwd\" in \"$here\"|\"$here\"/*) kill $pid && echo \"stopped pid $pid on :$port\"; found=1 ;; \
           *) echo \"left pid $pid on :$port alone (not this checkout: $cwd)\" ;; esac; \
         done; done; \
         [ $found = 1 ] || echo \"nothing from this checkout listens on :{p} or :{a}\""
    )
}

const NO_DOCKER: &str = "command -v docker >/dev/null 2>&1 || { echo 'docker is not installed: nothing to do'; exit 0; }";

fn rehearsal_present() -> String {
    format!(
        "{NO_DOCKER}; docker ps -a --format '{{{{.Names}}}}' | grep -qx {REHEARSAL_CONTAINER} \
         || {{ echo 'no rehearsal container'; exit 0; }}"
    )
}

/// The staging checkout, store and KEK directory of a plant target.
fn plant_paths(target: &Target) -> Result<(String, String, String), String> {
    let home = target.home.clone().unwrap_or_else(|| "~/staging".into());
    // `rm -rf` runs on these: never a home or root, never anything a
    // shell would split or expand beyond the leading `~/`.
    let (rest, depth) = match (home.strip_prefix("~/"), home.strip_prefix('/')) {
        (Some(rest), _) => (rest, 1),
        // An absolute home sits at least two levels down (/home/x/...).
        (None, Some(rest)) => (rest, 2),
        (None, None) => ("", 1),
    };
    let ok = rest.split('/').count() >= depth
        && !rest.split('/').any(|part| part.is_empty() || part == "..")
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b));
    if !ok {
        return Err(format!(
            "plant home {home:?} is not a plain directory under ~ (letters, digits, ._-/)"
        ));
    }
    Ok((
        format!("{home}/handbook"),
        format!("{home}/store"),
        format!("{home}/creds"),
    ))
}

fn stop_staging(repo: &str) -> String {
    format!(
        "repo=$(cd {repo} 2>/dev/null && pwd -P); found=0; \
         for pid in $(pgrep -f '^./target/release/web$'); do \
           if [ -n \"$repo\" ] && [ \"$(readlink /proc/$pid/cwd)\" = \"$repo\" ]; then kill $pid && echo \"stopped staging web ($pid)\"; found=1; fi; \
         done; [ $found = 1 ] || echo 'staging web is not running'"
    )
}

/// The steps that stop (or, with `remove`, stop and delete) one
/// non-server target; `Err` for a server or a refused path.
pub fn steps(file: &SetupFile, name: &str, action: All) -> Result<Vec<Step>, String> {
    let target = file
        .targets
        .get(name)
        .ok_or_else(|| format!("no target {name:?}"))?;
    let remove = action == All::Remove;
    let mut out = Vec::new();
    match target.kind {
        TargetKind::Server => {
            return Err(format!(
                "{name} is a server: the wizard never stops or removes one (roll back with the release tools)"
            ));
        }
        TargetKind::Local if target.runtime == Runtime::Docker => {
            let verb = if remove {
                "docker compose down --volumes --remove-orphans"
            } else {
                "docker compose stop"
            };
            out.push(plan::step(
                if remove { Phase::Remove } else { Phase::Stop },
                format!("{verb} (the judge image, compose.yaml)"),
                local(&format!("{NO_DOCKER}; {verb}")),
            ));
        }
        TargetKind::Local => {
            let ports = target.ports_or_default();
            out.push(plan::step(
                Phase::Stop,
                format!(
                    "stop the handbook on :{} and :{} (this checkout's processes only)",
                    ports.public, ports.admin
                ),
                local(&stop_ports([ports.public, ports.admin])),
            ));
            if remove {
                out.push(plan::step(
                    Phase::Remove,
                    "delete the local store, KEK and worker files: everything in .local except setup.yaml",
                    local(EMPTY_LOCAL),
                ));
            }
        }
        TargetKind::Docker => {
            if remove {
                out.push(plan::step(
                    Phase::Remove,
                    format!("delete the {REHEARSAL_CONTAINER} container (its key and CA stay)"),
                    local(&format!(
                        "{}; docker rm -f {REHEARSAL_CONTAINER} && rm -f {REHEARSAL_DIR}/known_hosts",
                        rehearsal_present()
                    )),
                ));
            } else {
                out.push(plan::step(
                    Phase::Stop,
                    format!("stop the {REHEARSAL_CONTAINER} container"),
                    local(&format!(
                        "{}; docker stop {REHEARSAL_CONTAINER}",
                        rehearsal_present()
                    )),
                ));
            }
        }
        TargetKind::Plant => {
            let alias = target.ssh.clone().ok_or("plant target needs ssh")?;
            let conn = Conn::alias(&alias);
            let (repo, store, creds) = plant_paths(target)?;
            out.push(plan::step(
                Phase::Stop,
                format!("stop staging web on {alias} (verified PID)"),
                plan::ssh(&conn, stop_staging(&repo), Feed::Nothing),
            ));
            if remove {
                out.push(plan::step(
                    Phase::Remove,
                    format!("delete {store} and {creds} on {alias} (the checkout stays)"),
                    plan::ssh(
                        &conn,
                        format!("rm -rf {store} {creds} && echo removed"),
                        Feed::Nothing,
                    ),
                ));
            }
        }
    }
    Ok(out)
}

/// One line per target for the page's confirmation: what the action
/// does there.
pub fn describe(file: &SetupFile, action: All) -> Vec<String> {
    local_targets(file)
        .iter()
        .map(|name| match steps(file, name, action) {
            Ok(steps) => format!(
                "{name}: {}",
                steps
                    .iter()
                    .map(|s| s.title.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            Err(e) => format!("{name}: skipped ({e})"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(step: &Step) -> &str {
        match &step.action {
            Action::Local { argv } => &argv[2],
            Action::Ssh { script, .. } => script,
            other => panic!("{other:?}"),
        }
    }

    /// The real script, on two throwaway listeners: the one started in
    /// "this checkout" stops, the one started elsewhere stays.
    #[cfg(unix)]
    #[test]
    fn stop_kills_this_checkouts_listener_and_leaves_others() {
        use std::process::{Command, Stdio};
        let have = |bin: &str, arg: &str| Command::new(bin).arg(arg).output().is_ok();
        if !have("python3", "--version") || !have("lsof", "-v") {
            return;
        }
        let (mine, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let port = || {
            std::net::TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port()
        };
        let (p_mine, p_other) = (port(), port());
        let serve = |dir: &std::path::Path, port: u16| {
            Command::new("python3")
                .args([
                    "-m",
                    "http.server",
                    &port.to_string(),
                    "--bind",
                    "127.0.0.1",
                ])
                .current_dir(dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        };
        let (mut a, mut b) = (serve(mine.path(), p_mine), serve(other.path(), p_other));
        let up = |port: u16| std::net::TcpStream::connect(("127.0.0.1", port)).is_ok();
        for _ in 0..100 {
            if up(p_mine) && up(p_other) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let out = Command::new("sh")
            .args(["-c", &stop_ports([p_mine, p_other])])
            .current_dir(mine.path())
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let mine_exited = (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            a.try_wait().unwrap().is_some()
        });
        let other_alive = b.try_wait().unwrap().is_none();
        let _ = a.kill();
        let _ = b.kill();
        let _ = (a.wait(), b.wait());
        assert!(out.status.success(), "{text}");
        assert!(mine_exited, "this checkout's listener still runs: {text}");
        assert!(
            other_alive,
            "another checkout's listener was killed: {text}"
        );
        assert!(
            text.contains(&format!("on :{p_mine}")) && text.contains("not this checkout"),
            "{text}"
        );
    }

    #[test]
    fn every_stop_and_remove_script_parses() {
        let f = SetupFile::template();
        for name in local_targets(&f) {
            for action in [All::Stop, All::Remove] {
                for step in steps(&f, &name, action).unwrap() {
                    let ok = std::process::Command::new("sh")
                        .args(["-n", "-c", script(&step)])
                        .status()
                        .unwrap()
                        .success();
                    assert!(ok, "{name} {action:?}: {}", script(&step));
                }
            }
        }
    }

    #[test]
    fn run_all_goes_staging_then_front_end_then_workers_once() {
        let f = SetupFile::template();
        assert_eq!(run_order(&f), vec!["staging", "production"]);
        let local = SetupFile::local_template();
        assert_eq!(run_order(&local), vec!["local"]);
    }

    #[test]
    fn stop_and_remove_never_touch_a_server() {
        let f = SetupFile::template();
        assert_eq!(local_targets(&f), vec!["docker", "local", "staging"]);
        assert_eq!(servers(&f), vec!["production"]);
        for action in [All::Stop, All::Remove] {
            let err = steps(&f, "production", action).unwrap_err();
            assert!(err.contains("never stops or removes"), "{err}");
        }
        assert!(
            describe(&f, All::Remove)
                .iter()
                .all(|line| !line.starts_with("production"))
        );
    }

    #[test]
    fn remove_keeps_the_setup_file_and_staging_checkout() {
        let f = SetupFile::template();
        let local = steps(&f, "local", All::Remove).unwrap();
        assert_eq!(local[0].phase, Phase::Stop);
        // The real script, on a scratch `.local/`.
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join(".local");
        std::fs::create_dir_all(data.join("store")).unwrap();
        for name in ["setup.yaml", "handbook-kek", "worker.enrol", ".npm-ci-hash"] {
            std::fs::write(data.join(name), "x").unwrap();
        }
        let ran = std::process::Command::new("sh")
            .args(["-c", script(&local[1])])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(ran.success());
        let left: Vec<_> = std::fs::read_dir(&data)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(left, vec!["setup.yaml"]);
        let staging = steps(&f, "staging", All::Remove).unwrap();
        let rm = script(&staging[1]);
        assert_eq!(rm, "rm -rf ~/staging/store ~/staging/creds && echo removed");
        assert_eq!(steps(&f, "staging", All::Stop).unwrap().len(), 1);
        let stop = script(&steps(&f, "local", All::Stop).unwrap()[0]).to_string();
        assert!(
            stop.contains("18318") && stop.contains("not this checkout"),
            "{stop}"
        );
        let docker = script(&steps(&f, "docker", All::Remove).unwrap()[0]).to_string();
        assert!(
            docker.contains("docker rm -f handbook-rehearsal"),
            "{docker}"
        );
        assert!(!docker.contains("id_ed25519"), "the rehearsal key stays");
    }

    #[test]
    fn a_plant_home_that_could_delete_too_much_is_refused() {
        for bad in [
            "~",
            "~/",
            "/",
            "",
            "/home",
            "staging",
            "~/a/../..",
            "~/a b",
            "~/$HOME",
            "~/a;rm",
        ] {
            let mut f = SetupFile::template();
            f.targets.get_mut("staging").unwrap().home = Some(bad.to_string());
            assert!(steps(&f, "staging", All::Remove).is_err(), "{bad:?}");
        }
        let mut f = SetupFile::template();
        for good in ["~/stage-2", "/home/chandra/staging"] {
            f.targets.get_mut("staging").unwrap().home = Some(good.into());
            assert!(steps(&f, "staging", All::Remove).is_ok(), "{good}");
        }
    }
}
