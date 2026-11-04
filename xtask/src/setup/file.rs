//! The setup file: every setting and secret for every target, in one
//! sops-encrypted YAML file (default `private/config.yaml`, git-ignored).
//!
//! Plaintext never touches disk: reads pipe `sops decrypt` into the YAML
//! parser, writes pipe the YAML into `sops encrypt --filename-override`
//! and move the ciphertext into place (0600, directory 0700). The age
//! recipient comes from the operator's age key (`private/.sops.yaml` is
//! written on first save).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA: u32 = 1;
pub const DEFAULT_PATH: &str = "private/config.yaml";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SetupFile {
    pub schema: u32,
    #[serde(default)]
    pub operator: Operator,
    /// Targets `cargo xtask setup` runs when no `--target` is given,
    /// in order (`[local]`, or `[staging, production]` for the split).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_targets: Vec<String>,
    /// Named targets: `local`, `docker`, `staging`, `production`, ...
    #[serde(default)]
    pub targets: BTreeMap<String, Target>,
    /// Registry config values shared by every target (`net.port`,
    /// `meta.api_key`, ...). Per-target values in `targets.<t>.config` win.
    #[serde(default)]
    pub config: BTreeMap<String, Value>,
    #[serde(default)]
    pub acme: Acme,
    #[serde(default)]
    pub brand: Brand,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Operator {
    /// Name recorded on licence acceptances and audit rows.
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    /// This machine, loopback only (the old `xtask up`).
    #[default]
    Local,
    /// A local Docker container that plays a production server.
    Docker,
    /// The build and staging box (user-level only, never root).
    Plant,
    /// A systemd server reached over SSH (production).
    Server,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub kind: TargetKind,
    /// SSH alias for unprivileged work (deploys as `hal` on servers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<String>,
    /// SSH alias of a sudo-capable account, used only to provision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_admin: Option<String>,
    /// The public host name; the admin host is `admin.<public_host>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind6: Option<String>,
    /// Names DNS must resolve to the bind IPs (also the certificate names).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dns: Vec<String>,
    /// Plain-HTTP ports (local and plant): public, admin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ports: Option<Ports>,
    /// Timers to enable on a server (`watch`, `mail`, `backup`, ...).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timers: Vec<String>,
    /// Docker: `auto`, `linux/arm64` or `linux/amd64`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Docker: host ports (on 127.0.0.1) for the container's 443 and 80.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish: Option<Publish>,
    /// Docker: host port mapped to the container's SSH.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<u16>,
    /// Public keys allowed to log in as hal (servers; default: the keys
    /// loaded in ssh-agent).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deploy_keys: Vec<String>,
    /// Plant: home of the staging stack (default `~/staging`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// Registry values for this target only (win over `config`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Ports {
    pub public: u16,
    pub admin: u16,
}

/// Plain-HTTP ports when a local or plant target names none.
pub const DEFAULT_PORTS: Ports = Ports {
    public: 18318,
    admin: 18319,
};

impl Target {
    /// This target's `ports`, or [`DEFAULT_PORTS`].
    pub fn ports_or_default(&self) -> Ports {
        self.ports.unwrap_or(DEFAULT_PORTS)
    }
}

/// Host ports a rehearsal container publishes (8443/8080 by default:
/// 443 and 80 on a developer machine are often taken).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Publish {
    pub https: u16,
    pub http: u16,
}

impl Default for Publish {
    fn default() -> Self {
        Publish {
            https: 8443,
            http: 8080,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Acme {
    /// Google Cloud project to mint a single-use EAB key from just in
    /// time (`gcloud publicca external-account-keys create`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gcloud_project: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Brand {
    /// Where licensed brand files live (default `private/brand`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub licences: Vec<Licence>,
}

/// One licence the operator accepted for a brand asset.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Licence {
    /// `apple`, `google`, `meta`, `fido`, `vogun`.
    pub asset: String,
    pub terms: String,
    #[serde(default)]
    pub terms_url: String,
    #[serde(default)]
    pub terms_version: String,
    pub accepted_by: String,
    /// RFC 3339 UTC.
    pub accepted_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<BrandFile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BrandFile {
    pub name: String,
    pub sha256: String,
}

impl SetupFile {
    /// A fresh file with the four standard targets and nothing secret.
    pub fn template() -> Self {
        let mut targets = BTreeMap::new();
        targets.insert(
            "local".to_string(),
            Target {
                kind: TargetKind::Local,
                ports: Some(Ports {
                    public: 18318,
                    admin: 18319,
                }),
                ..Target::default()
            },
        );
        targets.insert(
            "docker".to_string(),
            Target {
                kind: TargetKind::Docker,
                public_host: Some("rehearsal.localhost".to_string()),
                platform: Some("auto".to_string()),
                publish: Some(Publish::default()),
                ssh_port: Some(2222),
                timers: vec!["backup".into()],
                ..Target::default()
            },
        );
        targets.insert(
            "staging".to_string(),
            Target {
                kind: TargetKind::Plant,
                ssh: Some("plant".to_string()),
                home: Some("~/staging".to_string()),
                ports: Some(Ports {
                    public: 18318,
                    admin: 18319,
                }),
                ..Target::default()
            },
        );
        targets.insert(
            "production".to_string(),
            Target {
                kind: TargetKind::Server,
                ssh: Some("andrii.ai".to_string()),
                public_host: Some("andrii.ai".to_string()),
                dns: vec![
                    "andrii.ai".into(),
                    "admin.andrii.ai".into(),
                    "www.andrii.ai".into(),
                ],
                timers: vec!["watch".into(), "mail".into(), "backup".into()],
                ..Target::default()
            },
        );
        SetupFile {
            schema: SCHEMA,
            default_targets: vec!["local".to_string()],
            targets,
            ..SetupFile::default()
        }
    }

    /// Structural checks that need no network: schema, target shapes,
    /// registry keys and types, no secret reused across production and
    /// any other target.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if self.schema != SCHEMA {
            problems.push(format!("schema {} is not {SCHEMA}", self.schema));
        }
        let mut check_values = |scope: &str, values: &BTreeMap<String, Value>| {
            for (key, value) in values {
                if let Err(e) = registry_value(key, value) {
                    problems.push(format!("{scope}.{key}: {e}"));
                }
            }
        };
        check_values("config", &self.config);
        for (name, target) in &self.targets {
            check_values(&format!("targets.{name}.config"), &target.config);
        }
        for name in &self.default_targets {
            if !self.targets.contains_key(name) {
                problems.push(format!(
                    "default_targets names {name:?}, which is not a target"
                ));
            }
        }
        for (name, target) in &self.targets {
            for key in &target.deploy_keys {
                if !key.starts_with("ssh-") && !key.starts_with("ecdsa-") || key.contains('\n') {
                    problems.push(format!(
                        "targets.{name}.deploy_keys: not one OpenSSH public key line"
                    ));
                }
            }
            match target.kind {
                TargetKind::Plant | TargetKind::Server if target.ssh.is_none() => {
                    problems.push(format!("targets.{name}: needs an ssh alias"));
                }
                TargetKind::Server if target.public_host.is_none() => {
                    problems.push(format!("targets.{name}: needs public_host"));
                }
                _ => {}
            }
            if let Some(host) = &target.public_host
                && !valid_host(host)
            {
                problems.push(format!(
                    "targets.{name}.public_host {host:?} is not a host name"
                ));
            }
        }
        // Secrets are per target, so a production key can never serve a
        // test target by inheritance, and never by copy either.
        for key in self.config.keys() {
            if is_secret(key) {
                problems.push(format!(
                    "config.{key}: secrets go under targets.<name>.config, one value per target"
                ));
            }
        }
        if let Some(prod) = self.targets.get("production") {
            for (name, target) in &self.targets {
                if name == "production" {
                    continue;
                }
                for (key, value) in &target.config {
                    if is_secret(key) && prod.config.get(key) == Some(value) {
                        problems.push(format!(
                            "{key}: production and {name} share the same secret; give {name} its own test value"
                        ));
                    }
                }
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }

    /// Shared values overlaid with the target's own (no derived keys).
    pub fn effective_values(&self, _name: &str, target: &Target) -> BTreeMap<String, Value> {
        let mut out = self.config.clone();
        for (k, v) in &target.config {
            out.insert(k.clone(), v.clone());
        }
        out
    }

    /// The full config a target boots with: derived keys first, then
    /// shared values, then the target's own (later wins).
    pub fn target_config(&self, name: &str) -> Result<BTreeMap<String, Value>, String> {
        let target = self
            .targets
            .get(name)
            .ok_or_else(|| format!("no target {name:?} in the setup file"))?;
        let mut out = derived(target);
        for (k, v) in self.effective_values(name, target) {
            out.insert(k, v);
        }
        Ok(out)
    }

    /// The `config-import` payload for one target.
    pub fn import_payload(&self, name: &str) -> Result<Value, String> {
        let values = self.target_config(name)?;
        Ok(serde_json::json!({
            "format": crate::config_import::FORMAT,
            "values": values,
        }))
    }
}

/// Keys a target's shape decides; shown in the wizard, not typed.
pub fn derived(target: &Target) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let s = |v: &str| Value::String(v.to_string());
    match target.kind {
        TargetKind::Local | TargetKind::Plant => {
            let ports = target.ports_or_default();
            out.insert("net.bind".into(), s("127.0.0.1"));
            out.insert("net.port".into(), s(&ports.public.to_string()));
            out.insert("net.admin_port".into(), s(&ports.admin.to_string()));
        }
        TargetKind::Docker | TargetKind::Server => {
            let host = target
                .public_host
                .clone()
                .unwrap_or_else(|| "andrii.ai".into());
            out.insert("net.public_host".into(), s(&host));
            out.insert("auth.rp_id".into(), s(&host));
            // The browser's origin carries the published port when it is
            // not 443 (the rehearsal); admin.<host> on that port is a
            // subdomain origin of it.
            let https = match (target.kind, target.publish) {
                (TargetKind::Docker, Some(p)) if p.https != 443 => format!(":{}", p.https),
                (TargetKind::Docker, None) => ":8443".to_string(),
                _ => String::new(),
            };
            out.insert("auth.origin".into(), s(&format!("https://{host}{https}")));
            out.insert(
                "paths.static_dir".into(),
                s("/srv/apostille/current/static"),
            );
            out.insert("paths.spa_dir".into(), s("/srv/apostille/current/spa"));
            out.insert(
                "paths.admin_spa_dir".into(),
                s("/srv/apostille/current/spa-admin"),
            );
            out.insert(
                "paths.acme_challenge_dir".into(),
                s("/var/lib/apostille/acme-challenge"),
            );
            let names = if target.dns.is_empty() {
                vec![host.clone(), format!("admin.{host}")]
            } else {
                target.dns.clone()
            };
            out.insert("acme.domains".into(), s(&names.join(",")));
            if let Some(bind) = &target.bind {
                out.insert("net.bind".into(), s(bind));
            } else if target.kind == TargetKind::Docker {
                out.insert("net.bind".into(), s("0.0.0.0"));
            }
            if let Some(bind6) = &target.bind6 {
                out.insert("net.bind6".into(), s(bind6));
            }
            out.insert(
                "quic.enabled".into(),
                Value::Bool(target.kind == TargetKind::Server),
            );
            if target.kind == TargetKind::Docker {
                // Requests reach the container from Docker's bridge
                // gateway; its ports are published on 127.0.0.1 only.
                out.insert("admin.allow_ips".into(), s("172.16.0.0/12"));
                // TLS from the rehearsal CA (no public DNS for ACME).
                out.insert("acme.enabled".into(), Value::Bool(false));
                out.insert("tls.cert_file".into(), s("/etc/apostille/tls/cert.pem"));
                out.insert("tls.key_file".into(), s("/etc/apostille/tls/key.pem"));
            } else {
                out.insert("acme.enabled".into(), Value::Bool(true));
            }
        }
    }
    out
}

/// True for registry keys of type secret.
pub fn is_secret(key: &str) -> bool {
    sealed_config::keys::ALL
        .iter()
        .any(|(k, t)| *k == key && *t == sealed_config::ConfigType::Secret)
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 200
        && host
            .split('.')
            .all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
}

/// One registry value's shape (`config-import` checks it again on the
/// server; this catches mistakes before anything is sent).
pub fn registry_value(key: &str, value: &Value) -> Result<(), String> {
    use sealed_config::ConfigType;
    if key.starts_with("setup.") {
        return Err("minted on the server, never set here".into());
    }
    if sealed_config::keys::REMOVED.contains(&key) {
        return Err("removed from the registry".into());
    }
    let Some((_, kind)) = sealed_config::keys::ALL.iter().find(|(k, _)| *k == key) else {
        return Err("not a registered config key".into());
    };
    match (kind, value) {
        (ConfigType::Bool, Value::Bool(_)) => Ok(()),
        (ConfigType::Text, Value::String(_) | Value::Number(_)) => Ok(()),
        (ConfigType::Secret, Value::String(s)) if !s.is_empty() => Ok(()),
        (kind, _) => Err(format!("expected a {} value", kind.as_str())),
    }
}

// ------------------------------------------------------------- sops I/O

#[derive(Debug)]
pub enum LoadError {
    NotFound,
    Other(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::NotFound => write!(f, "setup file not found"),
            LoadError::Other(e) => write!(f, "{e}"),
        }
    }
}

/// Refuse a setup file git could pick up: inside the repo it must be
/// ignored and untracked (the `private/` directory is).
pub fn guard_path(path: &Path) -> Result<(), String> {
    let root = crate::repo_root();
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    if !abs.starts_with(&root) {
        return Ok(());
    }
    let rel = abs.strip_prefix(&root).map_err(|e| e.to_string())?;
    let tracked = Command::new("git")
        .args(["ls-files", "--error-unmatch"])
        .arg(rel)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if tracked {
        return Err(format!(
            "{} is tracked by git; keep it out of the repo",
            rel.display()
        ));
    }
    let ignored = Command::new("git")
        .args(["check-ignore", "-q"])
        .arg(rel)
        .current_dir(&root)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ignored {
        return Err(format!(
            "{} is inside the repo but not git-ignored; use private/ (ignored) or a path outside the repo",
            rel.display()
        ));
    }
    Ok(())
}

/// Decrypt and parse the setup file.
pub fn load(path: &Path) -> Result<SetupFile, LoadError> {
    if !path.exists() {
        return Err(LoadError::NotFound);
    }
    guard_path(path).map_err(LoadError::Other)?;
    let out = Command::new("sops")
        .args(["decrypt", "--input-type", "yaml", "--output-type", "yaml"])
        .arg(path)
        .output()
        .map_err(|e| LoadError::Other(format!("run sops (is it installed?): {e}")))?;
    if !out.status.success() {
        return Err(LoadError::Other(format!(
            "sops could not decrypt {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let text = String::from_utf8(out.stdout).map_err(|e| LoadError::Other(e.to_string()))?;
    serde_yaml_ng::from_str(&text).map_err(|e| LoadError::Other(format!("setup file: {e}")))
}

/// The operator's age recipient (public key), from the age key file sops uses.
pub fn age_recipient() -> Result<String, String> {
    let key_file = std::env::var("SOPS_AGE_KEY_FILE").unwrap_or_else(|_| {
        format!(
            "{}/.config/sops/age/keys.txt",
            std::env::var("HOME").unwrap_or_default()
        )
    });
    let out = Command::new("age-keygen")
        .arg("-y")
        .arg(&key_file)
        .output()
        .map_err(|e| format!("run age-keygen: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "no age key at {key_file} (create one with `age-keygen -o {key_file}`)"
        ));
    }
    let recipient = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !recipient.starts_with("age1") {
        return Err("age-keygen returned no recipient".into());
    }
    Ok(recipient)
}

/// Encrypt and write the setup file (0600; directory 0700).
pub fn save(path: &Path, file: &SetupFile) -> Result<(), String> {
    guard_path(path)?;
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    set_mode(&dir, 0o700);
    let sops_rules = dir.join(".sops.yaml");
    if !sops_rules.exists() {
        let recipient = age_recipient()?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config.yaml");
        std::fs::write(
            &sops_rules,
            format!(
                "creation_rules:\n  - path_regex: {}$\n    age: {recipient}\n",
                regex::escape(name)
            ),
        )
        .map_err(|e| e.to_string())?;
    }
    let yaml = serde_yaml_ng::to_string(file).map_err(|e| e.to_string())?;
    let mut child = Command::new("sops")
        .args([
            "encrypt",
            "--input-type",
            "yaml",
            "--output-type",
            "yaml",
            "--filename-override",
        ])
        .arg(path)
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("run sops (is it installed?): {e}"))?;
    {
        use std::io::Write as _;
        let mut stdin = child.stdin.take().ok_or("sops stdin")?;
        stdin
            .write_all(yaml.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "sops could not encrypt: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let tmp = dir.join(format!(".{}.tmp", std::process::id()));
    std::fs::write(&tmp, &out.stdout).map_err(|e| e.to_string())?;
    set_mode(&tmp, 0o600);
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_validates_and_derives_target_keys() {
        let file = SetupFile::template();
        file.validate().unwrap();
        let docker = file.target_config("docker").unwrap();
        assert_eq!(docker["net.public_host"], "rehearsal.localhost");
        assert_eq!(
            docker["acme.domains"],
            "rehearsal.localhost,admin.rehearsal.localhost"
        );
        assert_eq!(docker["acme.enabled"], false);
        let prod = file.target_config("production").unwrap();
        assert_eq!(
            prod["acme.domains"],
            "andrii.ai,admin.andrii.ai,www.andrii.ai"
        );
        assert_eq!(prod["auth.origin"], "https://andrii.ai");
        let local = file.target_config("local").unwrap();
        assert_eq!(local["net.bind"], "127.0.0.1");
        // Every derived key is a registered key with the right type.
        for target in ["local", "docker", "staging", "production"] {
            for (k, v) in file.target_config(target).unwrap() {
                registry_value(&k, &v).unwrap_or_else(|e| panic!("{target} {k}: {e}"));
            }
        }
    }

    #[test]
    fn bad_values_and_shared_production_secrets_are_refused() {
        let mut file = SetupFile::template();
        file.config
            .insert("quic.enabled".into(), Value::String("yes".into()));
        file.config
            .insert("tls.client_ca_file".into(), Value::String("/x".into()));
        file.config
            .insert("setup.key_hash".into(), Value::String("x".into()));
        let err = file.validate().unwrap_err().join("; ");
        for needle in ["quic.enabled", "tls.client_ca_file", "setup.key_hash"] {
            assert!(err.contains(needle), "{needle}: {err}");
        }
        let mut file = SetupFile::template();
        file.targets.get_mut("production").unwrap().config.insert(
            "stripe.secret_key".into(),
            Value::String(format!("sk_{}_synthetic", "live")),
        );
        file.validate().unwrap();
        file.targets.get_mut("staging").unwrap().config.insert(
            "stripe.secret_key".into(),
            Value::String(format!("sk_{}_synthetic", "live")),
        );
        let err = file.validate().unwrap_err().join("; ");
        assert!(err.contains("share the same secret"), "{err}");
        let mut file = SetupFile::template();
        file.config
            .insert("meta.api_key".into(), Value::String("LLM_1_shared".into()));
        let err = file.validate().unwrap_err().join("; ");
        assert!(err.contains("secrets go under targets"), "{err}");
    }

    #[test]
    fn yaml_round_trip_keeps_every_field() {
        let mut file = SetupFile::template();
        file.operator.name = "Synthetic Operator".into();
        file.config
            .insert("meta.daily_call_cap".into(), Value::String("400".into()));
        file.brand.licences.push(Licence {
            asset: "google".into(),
            terms: "Sign in with Google branding guidelines".into(),
            accepted_by: "Synthetic Operator".into(),
            accepted_at: "2026-11-03T18:00:00Z".into(),
            ..Licence::default()
        });
        let yaml = serde_yaml_ng::to_string(&file).unwrap();
        let back: SetupFile = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(back, file);
    }

    #[test]
    fn sops_round_trip_with_a_throwaway_age_key() {
        // Skipped where sops or age-keygen is missing (plant, CI boxes).
        let have = |bin: &str| Command::new(bin).arg("--version").output().is_ok();
        if !have("sops") || !have("age-keygen") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("age.txt");
        let ok = Command::new("age-keygen")
            .arg("-o")
            .arg(&key)
            .output()
            .unwrap();
        assert!(ok.status.success());
        // SAFETY of env: this test is the only one touching SOPS_AGE_KEY_FILE.
        unsafe { std::env::set_var("SOPS_AGE_KEY_FILE", &key) };
        let path = dir.path().join("config.yaml");
        let mut file = SetupFile::template();
        file.targets.get_mut("local").unwrap().config.insert(
            "meta.api_key".into(),
            Value::String("LLM_1_synthetic_secret".into()),
        );
        save(&path, &file).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("LLM_1_synthetic_secret"), "plaintext on disk");
        assert!(raw.contains("sops:"));
        assert_eq!(load(&path).unwrap(), file);
        unsafe { std::env::remove_var("SOPS_AGE_KEY_FILE") };
    }
}
