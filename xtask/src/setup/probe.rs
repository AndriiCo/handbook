//! The wizard's SSH box probe: what a host is, cached on its target.
//!
//! Keys only: `BatchMode=yes` (no password or passphrase prompt),
//! `StrictHostKeyChecking=yes` (an unknown host key fails; the wizard
//! never writes `known_hosts`) and 10 s to connect. The host is checked
//! by [`valid_host`] and passed as its own argv element after `--`; the
//! remote command is the fixed `sh -s`, reading the fixed [`SCRIPT`] on
//! stdin (so any login shell works), and nothing typed reaches a shell
//! on either side.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// What a probe found, as cached in `targets.<name>.probe`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    /// The ssh host or alias probed.
    pub via: String,
    /// When (RFC 3339 UTC).
    pub at: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub user: String,
    /// `/etc/os-release` PRETTY_NAME, or `sw_vers` on macOS.
    #[serde(default)]
    pub os: String,
    /// `uname -srm`.
    #[serde(default)]
    pub kernel: String,
    /// `uname -m` (the last word of `kernel`).
    #[serde(default)]
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u64>,
    /// Free space in the login user's home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_free_mb: Option<u64>,
    /// `nvidia-smi -L`, one entry per GPU (UUIDs dropped).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gpus: Vec<String>,
    /// `docker --version`, when Docker is installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker: Option<String>,
}

/// POSIX sh, one `key=value` per line; anything missing is left out.
pub const SCRIPT: &str = r#"echo "hostname=$(hostname 2>/dev/null || uname -n)"
echo "user=$(id -un)"
if [ -r /etc/os-release ]; then (. /etc/os-release; echo "os=$PRETTY_NAME")
elif command -v sw_vers >/dev/null 2>&1; then echo "os=$(sw_vers -productName) $(sw_vers -productVersion)"; fi
echo "kernel=$(uname -srm)"
echo "cpus=$(getconf _NPROCESSORS_ONLN 2>/dev/null)"
if [ -r /proc/meminfo ]; then echo "mem_kb=$(awk '/^MemTotal:/ {print $2}' /proc/meminfo)"
else echo "mem_bytes=$(sysctl -n hw.memsize 2>/dev/null)"; fi
echo "disk_free_kb=$(df -Pk "$HOME" 2>/dev/null | awk 'NR==2 {print $4}')"
if command -v nvidia-smi >/dev/null 2>&1; then nvidia-smi -L 2>/dev/null | sed 's/^/gpu=/'; fi
if command -v docker >/dev/null 2>&1; then echo "docker=$(docker --version 2>/dev/null)"; fi
"#;

/// A host name, address or ssh alias, optionally `user@`: letters,
/// digits and `.-_@:` only, never starting with `-` (so it can never
/// read as an ssh option).
pub fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_@:".contains(&b))
}

/// Remote output is untrusted: keep each field short.
fn clip(text: &str) -> String {
    text.trim().chars().take(200).collect()
}

/// Read [`SCRIPT`]'s output.
pub fn parse(via: &str, at: &str, text: &str) -> Probe {
    let mut probe = Probe {
        via: via.to_string(),
        at: at.to_string(),
        ..Probe::default()
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let number = || value.parse::<u64>().ok();
        match key {
            "hostname" => probe.hostname = clip(value),
            "user" => probe.user = clip(value),
            "os" => probe.os = clip(value),
            "kernel" => {
                probe.kernel = clip(value);
                probe.arch = clip(value.split_whitespace().last().unwrap_or_default());
            }
            "cpus" => probe.cpus = value.parse().ok().filter(|n| *n > 0),
            "mem_kb" => probe.ram_mb = number().map(|kb| kb / 1024),
            "mem_bytes" => probe.ram_mb = number().map(|b| b / (1024 * 1024)),
            "disk_free_kb" => probe.disk_free_mb = number().map(|kb| kb / 1024),
            "gpu" if !value.is_empty() && probe.gpus.len() < 16 => {
                let name = value.split(" (UUID").next().unwrap_or(value);
                probe.gpus.push(clip(name));
            }
            "docker" if !value.is_empty() => probe.docker = Some(clip(value)),
            _ => {}
        }
    }
    probe
}

/// What to say when ssh fails: its own words, plus the way out when
/// the host key is not known yet.
pub fn failure(host: &str, stderr: &str) -> String {
    let said: String = stderr.trim().chars().take(2000).collect();
    let said = if said.is_empty() {
        format!("ssh {host} failed")
    } else {
        said
    };
    if said.contains("Host key verification failed") && !said.contains("HAS CHANGED") {
        format!("{said}\nrun `ssh {host}` once in a terminal to accept its key")
    } else {
        said
    }
}

/// Probe `host` over ssh (keys only, 30 s in all).
pub fn run(host: &str, at: &str) -> Result<Probe, String> {
    let out = ssh_script(host, SCRIPT, Duration::from_secs(30))?;
    Ok(parse(host, at, &out))
}

/// Run a fixed POSIX sh `script` on `host` (fed on stdin to `sh -s`,
/// keys only, 10 s to connect, `limit` in all); its stdout.
pub fn ssh_script(host: &str, script: &str, limit: Duration) -> Result<String, String> {
    if !valid_host(host) {
        return Err(format!(
            "{host:?} is not a host name or ssh alias (letters, digits and .-_@: only)"
        ));
    }
    let mut child = Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ConnectTimeout=10",
            "--",
            host,
            "sh -s",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("run ssh: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A refused login closes the pipe early; its stderr says why.
        let _ = stdin.write_all(script.as_bytes());
    }
    // Drain stdout while waiting: a long answer would fill the pipe.
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut text = Vec::new();
        if let Some(mut out) = stdout {
            let _ = std::io::Read::read_to_end(&mut out, &mut text);
        }
        text
    });
    let deadline = Instant::now() + limit;
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("ssh {host}: no answer in {} s", limit.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let stdout = reader.join().unwrap_or_default();
    if !out.status.success() {
        return Err(failure(host, &String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_names_never_options_or_shell() {
        for ok in [
            "plant",
            "hal@andrii.ai",
            "10.0.0.5",
            "fe80::1",
            "build_box-2.lan",
        ] {
            assert!(valid_host(ok), "{ok}");
        }
        for bad in [
            "",
            "-oProxyCommand=sh",
            "-p",
            "plant box",
            "a;b",
            "$(id)",
            "`id`",
            "a/b",
            "a\nb",
            "host'",
            &"a".repeat(254),
        ] {
            assert!(!valid_host(bad), "{bad:?}");
        }
        assert!(run("-oProxyCommand=touch /tmp/x", "t").is_err());
    }

    #[test]
    fn parses_a_linux_gpu_box_and_a_mac() {
        let linux = "hostname=plant\nuser=andrii\nos=Ubuntu 24.04.1 LTS\n\
                     kernel=Linux 6.8.0-45-generic x86_64\ncpus=16\nmem_kb=65779884\n\
                     disk_free_kb=912345678\n\
                     gpu=GPU 0: NVIDIA GeForce RTX 3060 (UUID: GPU-0f0e0d0c-synthetic)\n\
                     docker=Docker version 27.3.1, build ce12230\nnoise without equals\n";
        let p = parse("plant", "2026-11-04T00:00:00Z", linux);
        assert_eq!(p.via, "plant");
        assert_eq!(p.hostname, "plant");
        assert_eq!(p.user, "andrii");
        assert_eq!(p.os, "Ubuntu 24.04.1 LTS");
        assert_eq!(p.kernel, "Linux 6.8.0-45-generic x86_64");
        assert_eq!(p.arch, "x86_64");
        assert_eq!(p.cpus, Some(16));
        assert_eq!(p.ram_mb, Some(64238));
        assert_eq!(p.disk_free_mb, Some(890962));
        assert_eq!(p.gpus, vec!["GPU 0: NVIDIA GeForce RTX 3060".to_string()]);
        assert_eq!(
            p.docker.as_deref(),
            Some("Docker version 27.3.1, build ce12230")
        );

        let mac = "hostname=Synthetic-Mac.local\nuser=op\nos=macOS 26.1\n\
                   kernel=Darwin 25.1.0 arm64\ncpus=12\nmem_bytes=38654705664\n\
                   disk_free_kb=\n";
        let p = parse("mac", "t", mac);
        assert_eq!(p.os, "macOS 26.1");
        assert_eq!(p.arch, "arm64");
        assert_eq!(p.ram_mb, Some(36864));
        assert_eq!(p.disk_free_mb, None, "empty value is unknown, not zero");
        assert!(p.gpus.is_empty() && p.docker.is_none());

        let long = format!("os={}\ncpus=zero\n", "x".repeat(500));
        let p = parse("h", "t", &long);
        assert_eq!(p.os.len(), 200, "remote text is clipped");
        assert_eq!(p.cpus, None);
    }

    #[test]
    fn an_unknown_host_key_says_how_to_accept_it() {
        let err = failure(
            "plant",
            "No ED25519 host key is known for plant and you have requested strict checking.\r\nHost key verification failed.\r\n",
        );
        assert!(err.starts_with("No ED25519 host key is known"), "{err}");
        assert!(
            err.ends_with("run `ssh plant` once in a terminal to accept its key"),
            "{err}"
        );
        let changed = failure(
            "plant",
            "@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @\nHost key verification failed.",
        );
        assert!(!changed.contains("accept its key"), "{changed}");
        assert_eq!(
            failure("plant", "Permission denied (publickey)."),
            "Permission denied (publickey)."
        );
    }
}
