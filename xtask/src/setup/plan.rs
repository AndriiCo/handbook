//! What `setup run` does for a target, as data.
//!
//! [`plan`] turns the setup file plus a few facts (SHA, CPU) into an
//! ordered list of steps. Steps are plain data, so `--dry-run` prints
//! exactly what a run would do and a golden test pins the production
//! plan. Secrets never appear in a step: the config payload and the
//! provision manifest are rendered at run time and fed on stdin.

use std::fmt::Write as _;

use super::build::Arch;
use super::file::{Runtime, SetupFile, Target, TargetKind};

/// Where the rehearsal keeps its SSH key, known hosts and TLS CA
/// (inside the git-ignored `private/`).
pub const REHEARSAL_DIR: &str = "private/rehearsal";
pub const REHEARSAL_CONTAINER: &str = "handbook-rehearsal";
pub const REHEARSAL_IMAGE: &str = "handbook-rehearsal:26.04";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Doctor,
    Gate,
    Dns,
    Build,
    Container,
    Provision,
    Seal,
    Deploy,
    Acme,
    Staff,
    Verify,
    Boot,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Doctor => "doctor",
            Phase::Gate => "gate",
            Phase::Dns => "dns",
            Phase::Build => "build",
            Phase::Container => "container",
            Phase::Provision => "provision",
            Phase::Seal => "seal",
            Phase::Deploy => "deploy",
            Phase::Acme => "acme",
            Phase::Staff => "staff",
            Phase::Verify => "verify",
            Phase::Boot => "boot",
        }
    }

    pub fn parse(text: &str) -> Option<Phase> {
        ALL_PHASES.iter().copied().find(|p| p.as_str() == text)
    }
}

pub const ALL_PHASES: [Phase; 12] = [
    Phase::Doctor,
    Phase::Gate,
    Phase::Dns,
    Phase::Build,
    Phase::Container,
    Phase::Provision,
    Phase::Seal,
    Phase::Deploy,
    Phase::Acme,
    Phase::Staff,
    Phase::Verify,
    Phase::Boot,
];

/// How to reach a host over SSH.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conn {
    /// An alias from `~/.ssh/config`, or `user@host`.
    pub dest: String,
    pub port: Option<u16>,
    /// Key and known-hosts files (the rehearsal's own, under `private/`).
    pub identity: Option<String>,
    pub known_hosts: Option<String>,
}

impl Conn {
    pub fn alias(alias: &str) -> Conn {
        Conn {
            dest: alias.to_string(),
            port: None,
            identity: None,
            known_hosts: None,
        }
    }

    fn rehearsal(user: &str, port: u16) -> Conn {
        Conn {
            dest: format!("{user}@127.0.0.1"),
            port: Some(port),
            identity: Some(format!("{REHEARSAL_DIR}/id_ed25519")),
            known_hosts: Some(format!("{REHEARSAL_DIR}/known_hosts")),
        }
    }

    /// Options shared by `ssh` and `scp` (scp spells the port `-P`).
    pub fn options(&self, port_flag: &str) -> Vec<String> {
        let mut out = vec![
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            "ConnectTimeout=15".to_string(),
        ];
        if let Some(port) = self.port {
            out.push(port_flag.to_string());
            out.push(port.to_string());
        }
        if let Some(identity) = &self.identity {
            out.extend([
                "-i".to_string(),
                identity.clone(),
                "-o".into(),
                "IdentitiesOnly=yes".into(),
            ]);
        }
        if let Some(known) = &self.known_hosts {
            out.extend([
                "-o".to_string(),
                format!("UserKnownHostsFile={known}"),
                "-o".to_string(),
                "StrictHostKeyChecking=accept-new".to_string(),
            ]);
        }
        out
    }

    fn label(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{port}", self.dest),
            None => self.dest.clone(),
        }
    }
}

/// What a step feeds the remote command on stdin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    Nothing,
    /// The target's `config-import` payload (holds secrets).
    ConfigPayload,
    /// The provision manifest (deploy keys, timers, rehearsal TLS pair).
    Manifest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Builtin {
    Doctor {
        tools: Vec<&'static str>,
    },
    /// Build web + xtask for the CPU, harden-check, stage the release dir.
    BuildRelease {
        arch: Arch,
    },
    /// Plant stamp `acceptance/<sha12>.json` with `ci_green: true`.
    GateStamp,
    /// Schema or migration changes since the live release ship only
    /// with `--migration-ack <fresh production backup>`.
    MigrationRail {
        conn: Conn,
        url: String,
    },
    /// Every name resolves to the target's bind addresses.
    DnsCheck {
        names: Vec<String>,
        bind: Option<String>,
        bind6: Option<String>,
    },
    RehearsalKeys,
    RehearsalTls {
        names: Vec<String>,
    },
    StartContainer {
        platform: String,
        /// Host ports for the container's 443 and 80.
        https: u16,
        http: u16,
    },
    WaitSsh {
        conn: Conn,
    },
    /// Poll a health URL until it names `+<version>`.
    WaitHealth {
        url: String,
        ca: Option<String>,
        timeout_secs: u64,
    },
    /// `xtask up` on this machine, with the file's values imported.
    LocalBoot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Local {
        argv: Vec<String>,
    },
    Ssh {
        conn: Conn,
        script: String,
        feed: Feed,
    },
    /// Copy a local directory (tar over ssh) into a remote one.
    Upload {
        conn: Conn,
        from: String,
        to: String,
    },
    Builtin(Builtin),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub phase: Phase,
    pub title: String,
    pub action: Action,
}

/// Facts a plan needs that the file does not hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub sha: String,
    pub arch: Arch,
    /// `--migration-ack`: a production backup under /var/lib/apostille
    /// that is the rollback for a release crossing a migration.
    pub migration_ack: Option<String>,
}

impl Facts {
    pub fn version(&self) -> &str {
        &self.sha[..self.sha.len().min(7)]
    }
}

/// Local directory a release for `version` and `arch` is staged in.
pub fn release_dir(version: &str, arch: Arch) -> String {
    format!("target/setup/{version}-{}", arch.triple())
}

fn step(phase: Phase, title: impl Into<String>, action: Action) -> Step {
    Step {
        phase,
        title: title.into(),
        action,
    }
}

fn ssh(conn: &Conn, script: impl Into<String>, feed: Feed) -> Action {
    Action::Ssh {
        conn: conn.clone(),
        script: script.into(),
        feed,
    }
}

const SERVICE_TOOLS: [&str; 5] = ["ssh", "scp", "tar", "curl", "npm"];

/// The ordered steps for one target.
pub fn plan(file: &SetupFile, name: &str, facts: &Facts) -> Result<Vec<Step>, String> {
    let target = file
        .targets
        .get(name)
        .ok_or_else(|| format!("no target {name:?} in the setup file"))?;
    Ok(match target.kind {
        // The judge path (P1-13): compose.yaml builds ops/judge/ and
        // publishes 127.0.0.1:18318/18319; every start is a fresh store.
        TargetKind::Local if target.runtime == Runtime::Docker => vec![
            step(
                Phase::Doctor,
                "docker and curl",
                Action::Builtin(Builtin::Doctor {
                    tools: vec!["docker", "curl"],
                }),
            ),
            step(
                Phase::Deploy,
                "build and start the judge image (compose.yaml); its setup key: `docker compose logs judge`",
                Action::Local {
                    argv: ["docker", "compose", "up", "--build", "--detach"]
                        .map(String::from)
                        .to_vec(),
                },
            ),
            step(
                Phase::Verify,
                "healthz on http://localhost:18318/apostille (admin http://localhost:18319)",
                Action::Local {
                    argv: [
                        "sh",
                        "-c",
                        "for i in $(seq 1 300); do curl -fsS --max-time 2 http://localhost:18318/apostille/healthz && exit 0; sleep 1; done; exit 1",
                    ]
                    .map(String::from)
                    .to_vec(),
                },
            ),
        ],
        TargetKind::Local => vec![
            step(
                Phase::Doctor,
                "toolchain (rust, node, npm)",
                Action::Builtin(Builtin::Doctor {
                    tools: vec!["cargo", "npm"],
                }),
            ),
            step(
                Phase::Boot,
                "boot the handbook on loopback with this file's values",
                Action::Builtin(Builtin::LocalBoot),
            ),
        ],
        TargetKind::Docker => docker_plan(target, facts),
        TargetKind::Plant => plant_plan(target, facts)?,
        TargetKind::Server => server_plan(target, facts)?,
    })
}

fn names(target: &Target) -> Vec<String> {
    let host = target.public_host.clone().unwrap_or_default();
    if target.dns.is_empty() {
        vec![host.clone(), format!("admin.{host}")]
    } else {
        target.dns.clone()
    }
}

fn provision_steps(admin: &Conn, sudo: &str, home: &str, facts: &Facts) -> Vec<Step> {
    let v = facts.version();
    let dir = format!("{home}/setup/{v}");
    vec![
        step(
            Phase::Provision,
            format!("upload release {v} for provisioning"),
            Action::Upload {
                conn: admin.clone(),
                from: release_dir(v, facts.arch),
                to: dir.clone(),
            },
        ),
        step(
            Phase::Provision,
            "upload the provision manifest (0600)",
            ssh(
                admin,
                format!("umask 077 && cat > {home}/setup/manifest.json"),
                Feed::Manifest,
            ),
        ),
        step(
            Phase::Provision,
            "provision: users, layout, KEK, wrappers, sudoers, units, firewall, first install",
            ssh(
                admin,
                format!(
                    "{sudo}{dir}/xtask provision --manifest {home}/setup/manifest.json --release {dir} \
                     --version {v} --sha {} --config-stdin; rc=$?; rm -f {home}/setup/manifest.json; exit $rc",
                    facts.sha
                ),
                Feed::ConfigPayload,
            ),
        ),
    ]
}

/// Steps that run as hal once a server is provisioned: stage, seal,
/// deploy (skipped when the release is already live), first staff.
fn hal_steps(hal: &Conn, facts: &Facts, deploy_flags: &str) -> Vec<Step> {
    let v = facts.version();
    let mut steps = vec![
        step(
            Phase::Seal,
            "seal this file's config (merge; one transaction, boot-load checked)",
            ssh(
                hal,
                "sudo -n -u andrii-ai /usr/local/sbin/apostille-config-import",
                Feed::ConfigPayload,
            ),
        ),
        step(
            Phase::Deploy,
            format!("stage release {v} as hal"),
            Action::Upload {
                conn: hal.clone(),
                from: release_dir(v, facts.arch),
                to: format!("apostille/staging/{v}"),
            },
        ),
        step(
            Phase::Deploy,
            "install the operator scripts from the staged release",
            ssh(
                hal,
                format!(
                    "mkdir -p ~/apostille/scripts && cp ~/apostille/staging/{v}/scripts/*.sh ~/apostille/scripts/"
                ),
                Feed::Nothing,
            ),
        ),
    ];
    if let Some(ack) = &facts.migration_ack {
        // ops-deploy.sh forwards MIGRATION_NOTES into the release manifest.
        steps.push(step(
            Phase::Deploy,
            "record the migration rollback in the staged release",
            ssh(
                hal,
                format!(
                    "printf '%s\\n' 'rollback = restore {ack} (no older binary opens the migrated store)' \
                     > ~/apostille/staging/{v}/MIGRATION_NOTES"
                ),
                Feed::Nothing,
            ),
        ));
    }
    steps.push(
        step(
            Phase::Deploy,
            format!("deploy {v} unless it is already live (restarts on the sealed config)"),
            ssh(
                hal,
                format!(
                    "if [ \"$(readlink /srv/apostille/current)\" = /srv/apostille/releases/{v} ]; then \
                     echo \"{v} is live (first install)\"; sudo -n systemctl restart apostille; \
                     else bash ~/apostille/scripts/ops-deploy.sh ~/apostille/staging/{v}{deploy_flags}; fi"
                ),
                Feed::Nothing,
            ),
        ),
    );
    steps
}

fn staff_step(hal: &Conn) -> Step {
    step(
        Phase::Staff,
        "mint the first-boot setup key (prints once; refused once staff exist)",
        ssh(
            hal,
            "sudo -n -u andrii-ai /usr/local/sbin/apostille-setup-key || echo 'setup key not minted (staff exist)'",
            Feed::Nothing,
        ),
    )
}

fn docker_plan(target: &Target, facts: &Facts) -> Vec<Step> {
    let port = target.ssh_port.unwrap_or(2222);
    let root = Conn::rehearsal("root", port);
    let hal = Conn::rehearsal("hal", port);
    let host = target
        .public_host
        .clone()
        .unwrap_or_else(|| "rehearsal.localhost".into());
    let publish = target.publish.unwrap_or_default();
    let platform = match target.platform.as_deref() {
        None | Some("auto") => facts.arch.docker_platform().to_string(),
        Some(p) => p.to_string(),
    };
    let mut tools = vec!["docker", "openssl", "ssh-keygen"];
    tools.extend(SERVICE_TOOLS);
    let mut steps = vec![
        step(
            Phase::Doctor,
            "docker, ssh, openssl, npm, cross toolchain",
            Action::Builtin(Builtin::Doctor { tools }),
        ),
        step(
            Phase::Build,
            format!("build {} for {}", facts.version(), facts.arch.triple()),
            Action::Builtin(Builtin::BuildRelease { arch: facts.arch }),
        ),
        step(
            Phase::Container,
            "rehearsal SSH key (private/rehearsal)",
            Action::Builtin(Builtin::RehearsalKeys),
        ),
        step(
            Phase::Container,
            "rehearsal TLS CA and certificate",
            Action::Builtin(Builtin::RehearsalTls {
                names: names(target),
            }),
        ),
        step(
            Phase::Container,
            format!("start {REHEARSAL_CONTAINER} ({platform}, systemd as PID 1)"),
            Action::Builtin(Builtin::StartContainer {
                platform,
                https: publish.https,
                http: publish.http,
            }),
        ),
        step(
            Phase::Container,
            "wait for SSH",
            Action::Builtin(Builtin::WaitSsh { conn: root.clone() }),
        ),
    ];
    steps.extend(provision_steps(&root, "", "/root", facts));
    steps.extend(hal_steps(
        &hal,
        facts,
        " --skip-accept --allow-today-overlap",
    ));
    steps.push(step(
        Phase::Verify,
        "healthz names the release over the rehearsal CA",
        Action::Builtin(Builtin::WaitHealth {
            url: format!("https://{host}:{}/apostille/healthz", publish.https),
            ca: Some(format!("{REHEARSAL_DIR}/ca.pem")),
            timeout_secs: 90,
        }),
    ));
    steps.push(step(
        Phase::Verify,
        "service and timers active",
        ssh(
            &root,
            "systemctl is-active apostille && systemctl list-timers 'apostille-*' --no-pager --no-legend",
            Feed::Nothing,
        ),
    ));
    steps.push(staff_step(&hal));
    steps
}

fn plant_plan(target: &Target, facts: &Facts) -> Result<Vec<Step>, String> {
    let alias = target.ssh.clone().ok_or("plant target needs ssh")?;
    let conn = Conn::alias(&alias);
    let v = facts.version();
    let sha = &facts.sha;
    let home = target.home.clone().unwrap_or_else(|| "~/staging".into());
    let port = target.ports.map_or(18318, |p| p.public);
    let repo = format!("{home}/handbook");
    let kek = format!("{home}/creds/handbook-kek");
    let store = format!("{home}/store");
    Ok(vec![
        step(
            Phase::Doctor,
            "ssh and git",
            Action::Builtin(Builtin::Doctor {
                tools: vec!["ssh", "git"],
            }),
        ),
        step(
            Phase::Build,
            format!("push {v} to plant (no fetch: plant has no GitHub credentials)"),
            Action::Local {
                argv: vec![
                    "git".into(),
                    "push".into(),
                    format!("{alias}:staging/handbook"),
                    format!("{sha}:refs/heads/xfer-setup-{v}"),
                ],
            },
        ),
        step(
            Phase::Build,
            format!("build {v} on plant (sccache when present) and harden-check"),
            ssh(
                &conn,
                format!(
                    "set -e; cd {repo} && git checkout -qf {sha} && \
                     if command -v sccache >/dev/null; then export RUSTC_WRAPPER=sccache; fi && \
                     cargo build --release -q -p web -p xtask && \
                     ./target/release/xtask harden-check target/release/web target/release/xtask && \
                     npm --prefix web ci --no-audit --no-fund >/dev/null && \
                     npm --prefix web run build >/dev/null && npm --prefix web run build:admin >/dev/null"
                ),
                Feed::Nothing,
            ),
        ),
        step(
            Phase::Seal,
            "store, KEK (0600, created once) and this file's config",
            ssh(
                &conn,
                format!(
                    "set -e; mkdir -p {home}/creds {store} && chmod 700 {home}/creds && \
                     if [ ! -s {kek} ]; then (umask 077 && head -c 32 /dev/urandom > {kek}); fi && \
                     cd {repo} && ./target/release/xtask init --store {store} >/dev/null && \
                     ./target/release/xtask config-import --store {store} --kek-file {kek} --stdin \
                     --mode merge --enroll-if-empty --actor setup && \
                     ./target/release/xtask packs-load --store {store} >/dev/null"
                ),
                Feed::ConfigPayload,
            ),
        ),
        step(
            Phase::Deploy,
            "restart the staging stack by verified PID (setsid -f)",
            ssh(
                &conn,
                format!(
                    "PID=$(pgrep -f '^./target/release/web$' | head -1); \
                     if [ -n \"$PID\" ] && [ \"$(readlink /proc/$PID/cwd)\" = \"$(cd {repo} && pwd)\" ]; then kill $PID; sleep 2; fi; \
                     cd {repo} && if [ -f ~/.config/apostille/meta-staging.env ]; then set -a; . ~/.config/apostille/meta-staging.env; set +a; fi; \
                     STORE_DIR=$(cd {store} && pwd) CREDENTIALS_DIRECTORY=$(cd {home}/creds && pwd) \
                     SPA_DIR=$PWD/web/build STATIC_DIR=$PWD/crates/web/static APOSTILLE_ENV_LABEL=staging \
                     setsid -f ./target/release/web > /tmp/staging-web-{v}.log 2>&1 < /dev/null"
                ),
                Feed::Nothing,
            ),
        ),
        step(
            Phase::Verify,
            format!("staging healthz names {v}"),
            ssh(
                &conn,
                format!(
                    "for i in $(seq 1 60); do B=$(curl -fsS --max-time 2 http://127.0.0.1:{port}/apostille/healthz 2>/dev/null) && break; sleep 1; done; \
                     echo \"$B\"; echo \"$B\" | grep -q '+{v}'"
                ),
                Feed::Nothing,
            ),
        ),
    ])
}

/// The root-process probe: must print nothing (an operator script or
/// hidden root processes stop the run).
fn probe(hal: &Conn, phase: Phase) -> Step {
    step(
        phase,
        "root-process probe (no operator script may be running)",
        ssh(
            hal,
            "out=$( (pgrep -u 0 -c . >/dev/null || echo ROOT-PROCS-HIDDEN); pgrep -af '[a]postille/scripts/'); \
             [ -z \"$out\" ] || { echo \"$out\"; exit 1; }",
            Feed::Nothing,
        ),
    )
}

fn server_plan(target: &Target, facts: &Facts) -> Result<Vec<Step>, String> {
    let alias = target.ssh.clone().ok_or("server target needs ssh")?;
    let host = target
        .public_host
        .clone()
        .ok_or("server target needs public_host")?;
    let hal = Conn::alias(&alias);
    let admin = Conn::alias(target.ssh_admin.as_deref().unwrap_or(&alias));
    let mut tools = vec!["zig", "cargo-zigbuild"];
    tools.extend(SERVICE_TOOLS);
    let mut steps = vec![
        step(
            Phase::Doctor,
            "ssh aliases, cross toolchain, npm",
            Action::Builtin(Builtin::Doctor { tools }),
        ),
        step(
            Phase::Gate,
            format!("plant stamp for {} (ci_green)", facts.version()),
            Action::Builtin(Builtin::GateStamp),
        ),
        step(
            Phase::Gate,
            "schema and migration rail against the live release",
            Action::Builtin(Builtin::MigrationRail {
                conn: hal.clone(),
                url: format!("https://{host}/apostille/healthz"),
            }),
        ),
        probe(&hal, Phase::Gate),
        step(
            Phase::Gate,
            "scoreboard before the deploy",
            Action::Local {
                argv: scoreboard("web/shots/prod-before.json"),
            },
        ),
        step(
            Phase::Dns,
            "every certificate name resolves to this server",
            Action::Builtin(Builtin::DnsCheck {
                names: names(target),
                bind: target.bind.clone(),
                bind6: target.bind6.clone(),
            }),
        ),
        step(
            Phase::Build,
            format!("build {} for {}", facts.version(), facts.arch.triple()),
            Action::Builtin(Builtin::BuildRelease { arch: facts.arch }),
        ),
    ];
    steps.extend(provision_steps(&admin, "sudo -n ", "~", facts));
    let mut hal_part = hal_steps(&hal, facts, " --skip-accept");
    // Checked again right before the deploy itself (docs/ops/deploy.md).
    let deploy_at = hal_part.len() - 1;
    hal_part.insert(deploy_at, probe(&hal, Phase::Deploy));
    steps.extend(hal_part);
    steps.push(step(
        Phase::Acme,
        format!(
            "certificate issued and healthz names {} on https://{host}",
            facts.version()
        ),
        Action::Builtin(Builtin::WaitHealth {
            url: format!("https://{host}/apostille/healthz"),
            ca: None,
            timeout_secs: 600,
        }),
    ));
    steps.push(staff_step(&hal));
    steps.push(step(
        Phase::Verify,
        "production scoreboard",
        Action::Local {
            argv: scoreboard("web/shots/prod-after.json"),
        },
    ));
    Ok(steps)
}

fn scoreboard(out: &str) -> Vec<String> {
    ["node", "xtask/ops/scoreboard.mjs", "--report", "--out", out]
        .map(String::from)
        .to_vec()
}

/// The dry-run text: one block per step, commands exactly as run.
pub fn render(steps: &[Step]) -> String {
    let mut out = String::new();
    for (i, s) in steps.iter().enumerate() {
        let _ = writeln!(out, "{:>2}. [{}] {}", i + 1, s.phase.as_str(), s.title);
        match &s.action {
            Action::Local { argv } => {
                let _ = writeln!(out, "      $ {}", argv.join(" "));
            }
            Action::Ssh { conn, script, feed } => {
                let _ = writeln!(out, "      ssh {} -- {script}", conn.label());
                if *feed != Feed::Nothing {
                    let _ = writeln!(
                        out,
                        "      stdin: {feed:?} (rendered at run time, never printed)"
                    );
                }
            }
            Action::Upload { conn, from, to } => {
                let _ = writeln!(out, "      upload {from}/ -> {}:{to}/", conn.label());
            }
            Action::Builtin(b) => {
                let _ = writeln!(out, "      builtin {b:?}");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            sha: "0123456789abcdef0123456789abcdef01234567".into(),
            arch: Arch::X86_64,
            migration_ack: None,
        }
    }

    /// Golden: the production plan, step by step. A change here is a
    /// change to how production is rebuilt; review it as one.
    #[test]
    fn production_plan_is_pinned() {
        let file = SetupFile::template();
        let text = render(&plan(&file, "production", &facts()).unwrap());
        let golden = include_str!("testdata/production-plan.txt");
        if text != golden {
            let out = std::env::temp_dir().join("production-plan.txt");
            std::fs::write(&out, &text).unwrap();
            panic!("production plan changed; new text in {}", out.display());
        }
    }

    #[test]
    fn a_migration_ack_becomes_the_release_notes() {
        let file = SetupFile::template();
        let mut facts = facts();
        facts.migration_ack = Some("/var/lib/apostille/pre-014.sqlite".into());
        let text = render(&plan(&file, "production", &facts).unwrap());
        assert!(text.contains(
            "'rollback = restore /var/lib/apostille/pre-014.sqlite (no older binary opens the migrated store)' \
             > ~/apostille/staging/0123456/MIGRATION_NOTES"
        ));
        let notes = text.find("MIGRATION_NOTES").unwrap();
        assert!(notes < text.find("ops-deploy.sh ~/apostille/staging").unwrap());
    }

    #[test]
    fn plans_never_carry_secret_values_and_order_phases() {
        let mut file = SetupFile::template();
        for t in file.targets.values_mut() {
            t.config.insert(
                "meta.api_key".into(),
                serde_json::json!("LLM_1_synthetic_never_printed"),
            );
        }
        for name in ["local", "docker", "staging", "production"] {
            let steps = plan(&file, name, &facts()).unwrap();
            let text = render(&steps);
            assert!(
                !text.contains("LLM_1_synthetic"),
                "{name} plan leaks a secret"
            );
            let order: Vec<usize> = steps
                .iter()
                .map(|s| ALL_PHASES.iter().position(|p| *p == s.phase).unwrap())
                .collect();
            // Staff minting comes after verify on purpose (docker); all
            // else runs in phase order.
            let mut sorted = order.clone();
            sorted.sort_unstable();
            if name != "docker" {
                assert_eq!(order, sorted, "{name} phases out of order");
            }
        }
        let docker = render(&plan(&file, "docker", &facts()).unwrap());
        assert!(
            docker.contains("linux/amd64"),
            "auto platform follows the CPU"
        );
        assert!(docker.contains("127.0.0.1:2222"));
        // A local target in Docker runs the judge image instead of `up`.
        file.targets.get_mut("local").unwrap().runtime = Runtime::Docker;
        let steps = plan(&file, "local", &facts()).unwrap();
        let local = render(&steps);
        assert!(
            local.contains("docker compose up --build --detach"),
            "{local}"
        );
        assert!(!local.contains("LLM_1_synthetic"));
        let phases: Vec<Phase> = steps.iter().map(|s| s.phase).collect();
        assert_eq!(phases, [Phase::Doctor, Phase::Deploy, Phase::Verify]);
    }
}
