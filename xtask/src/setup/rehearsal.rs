//! The Docker rehearsal: an `ubuntu:26.04` container with systemd as
//! PID 1 that plays a blank production server on this machine.
//!
//! Everything it needs stays under the git-ignored `private/rehearsal/`:
//! its own SSH key (root logs in with it; `hal` gets it as a deploy
//! key), its known-hosts file, and a throwaway TLS CA whose leaf covers
//! `rehearsal.localhost` and `admin.rehearsal.localhost`. Browsers and
//! curl resolve `*.localhost` to loopback, so the container's 443 and
//! 80 are published on 127.0.0.1 only.

use std::path::Path;
use std::process::{Command, Stdio};

use super::plan::{Conn, REHEARSAL_CONTAINER, REHEARSAL_DIR, REHEARSAL_IMAGE};

fn run(cmd: &mut Command, what: &str) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("{what}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{what}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn ensure_dir(repo: &Path) -> Result<std::path::PathBuf, String> {
    let dir = repo.join(REHEARSAL_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

/// The rehearsal's own SSH key (created once, no passphrase: it opens
/// only a container on loopback).
pub fn ensure_keys(repo: &Path) -> Result<String, String> {
    let dir = ensure_dir(repo)?;
    let key = dir.join("id_ed25519");
    if !key.exists() {
        run(
            Command::new("ssh-keygen")
                .args([
                    "-q",
                    "-t",
                    "ed25519",
                    "-N",
                    "",
                    "-C",
                    "handbook-rehearsal",
                    "-f",
                ])
                .arg(&key),
            "ssh-keygen",
        )?;
    }
    std::fs::read_to_string(dir.join("id_ed25519.pub"))
        .map(|s| s.trim().to_string())
        .map_err(|e| e.to_string())
}

/// CA + leaf for `names` (openssl CLI; P-256, 30 days). The CA key never
/// leaves `private/rehearsal/`; the leaf pair goes into the manifest.
pub fn ensure_tls(repo: &Path, names: &[String]) -> Result<(String, String), String> {
    let dir = ensure_dir(repo)?;
    let (ca, ca_key, cert, key) = (
        dir.join("ca.pem"),
        dir.join("ca-key.pem"),
        dir.join("cert.pem"),
        dir.join("key.pem"),
    );
    if !ca.exists() || !ca_key.exists() {
        std::fs::write(
            dir.join("ca.cnf"),
            "[req]\ndistinguished_name=dn\nx509_extensions=v3\nprompt=no\n[dn]\nCN=Handbook rehearsal CA\n\
             [v3]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n\
             subjectKeyIdentifier=hash\n",
        )
        .map_err(|e| e.to_string())?;
        run(
            Command::new("openssl")
                .args([
                    "req",
                    "-x509",
                    "-newkey",
                    "ec",
                    "-pkeyopt",
                    "ec_paramgen_curve:P-256",
                    "-nodes",
                    "-days",
                    "30",
                ])
                .arg("-keyout")
                .arg(&ca_key)
                .arg("-out")
                .arg(&ca)
                .arg("-config")
                .arg(dir.join("ca.cnf")),
            "openssl CA",
        )?;
    }
    let sans = names
        .iter()
        .map(|n| format!("DNS:{n}"))
        .collect::<Vec<_>>()
        .join(",");
    let ext = dir.join("leaf.cnf");
    std::fs::write(
        &ext,
        format!(
            "basicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\n\
             subjectAltName={sans}\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    let leaf_names = dir.join("leaf.names");
    let fresh = std::fs::read_to_string(&leaf_names).is_ok_and(|s| s == sans)
        && cert.exists()
        && key.exists();
    if !fresh {
        let csr = dir.join("leaf.csr");
        let cn = names
            .first()
            .cloned()
            .unwrap_or_else(|| "rehearsal.localhost".into());
        run(
            Command::new("openssl")
                .args([
                    "req",
                    "-new",
                    "-newkey",
                    "ec",
                    "-pkeyopt",
                    "ec_paramgen_curve:P-256",
                    "-nodes",
                ])
                .arg("-keyout")
                .arg(&key)
                .arg("-out")
                .arg(&csr)
                .args(["-subj", &format!("/CN={cn}")]),
            "openssl leaf key",
        )?;
        run(
            Command::new("openssl")
                .args(["x509", "-req", "-days", "30", "-sha256", "-set_serial"])
                .arg(format!("{}", crate::unix_now()))
                .arg("-in")
                .arg(&csr)
                .arg("-CA")
                .arg(&ca)
                .arg("-CAkey")
                .arg(&ca_key)
                .arg("-extfile")
                .arg(&ext)
                .arg("-out")
                .arg(&cert),
            "openssl leaf sign",
        )?;
        let _ = std::fs::remove_file(&csr);
        std::fs::write(&leaf_names, &sans).map_err(|e| e.to_string())?;
    }
    let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
    Ok((read(&cert)?, read(&key)?))
}

fn container_running() -> bool {
    Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", REHEARSAL_CONTAINER])
        .output()
        .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
}

/// Build the image and start the container (kept when already running:
/// `setup wipe --target docker` removes it for a from-scratch run).
pub fn start_container(
    repo: &Path,
    platform: &str,
    names: &[String],
    log: &dyn Fn(String),
) -> Result<(), String> {
    run(
        Command::new("docker")
            .args(["build", "-q", "--platform", platform, "-t", REHEARSAL_IMAGE])
            .arg(repo.join("ops/rehearsal")),
        "docker build",
    )?;
    if container_running() {
        log(format!(
            "{REHEARSAL_CONTAINER} already running (wipe it for a from-scratch run)"
        ));
    } else {
        let _ = Command::new("docker")
            .args(["rm", "-f", REHEARSAL_CONTAINER])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let mut cmd = Command::new("docker");
        cmd.args([
            "run",
            "-d",
            "--name",
            REHEARSAL_CONTAINER,
            "--hostname",
            "rehearsal",
            "--platform",
            platform,
            "--privileged",
            "--cgroupns=host",
            "-v",
            "/sys/fs/cgroup:/sys/fs/cgroup:rw",
            "--tmpfs",
            "/run",
            "--tmpfs",
            "/run/lock",
            "-p",
            "127.0.0.1:2222:22",
            "-p",
            "127.0.0.1:443:443",
            "-p",
            "127.0.0.1:443:443/udp",
            "-p",
            "127.0.0.1:80:80",
        ]);
        for name in names {
            cmd.args(["--add-host", &format!("{name}:127.0.0.1")]);
        }
        cmd.arg(REHEARSAL_IMAGE);
        run(&mut cmd, "docker run")?;
        // A new container has a new host key.
        let _ = std::fs::remove_file(repo.join(REHEARSAL_DIR).join("known_hosts"));
        log(format!("started {REHEARSAL_CONTAINER} ({platform})"));
    }
    let pubkey = std::fs::read(repo.join(REHEARSAL_DIR).join("id_ed25519.pub"))
        .map_err(|e| e.to_string())?;
    let mut child = Command::new("docker")
        .args([
            "exec",
            "-i",
            REHEARSAL_CONTAINER,
            "sh",
            "-c",
            "umask 077 && mkdir -p /root/.ssh && cat > /root/.ssh/authorized_keys",
        ])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("docker exec: {e}"))?;
    {
        use std::io::Write as _;
        child
            .stdin
            .take()
            .ok_or("stdin")?
            .write_all(&pubkey)
            .map_err(|e| e.to_string())?;
    }
    if !child.wait().map_err(|e| e.to_string())?.success() {
        return Err("install the rehearsal key for root".into());
    }
    // The container trusts its CA, so the deploy smoke (curl) verifies TLS.
    run(
        Command::new("docker")
            .arg("cp")
            .arg(repo.join(REHEARSAL_DIR).join("ca.pem"))
            .arg(format!(
                "{REHEARSAL_CONTAINER}:/usr/local/share/ca-certificates/handbook-rehearsal.crt"
            )),
        "docker cp CA",
    )?;
    run(
        Command::new("docker").args(["exec", REHEARSAL_CONTAINER, "update-ca-certificates"]),
        "update-ca-certificates",
    )?;
    Ok(())
}

/// Remove the container (its whole disk goes with it) and the known
/// host entry. The key and CA stay, so a rerun needs no new trust.
pub fn wipe(repo: &Path) -> Result<(), String> {
    let _ = Command::new("docker")
        .args(["rm", "-f", REHEARSAL_CONTAINER])
        .status();
    let _ = std::fs::remove_file(repo.join(REHEARSAL_DIR).join("known_hosts"));
    println!("wiped {REHEARSAL_CONTAINER}");
    Ok(())
}

/// Poll until SSH answers (`true` as the connection's user).
pub fn wait_ssh(conn: &Conn, secs: u64) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let ok = Command::new("ssh")
            .args(conn.options("-p"))
            .arg(&conn.dest)
            .arg("true")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            return Err(format!("ssh {} never answered", conn.dest));
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
