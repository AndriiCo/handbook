//! `cargo xtask setup pull`: what a server target's store holds now.
//!
//! Over SSH as the target's unprivileged user (hal), through the same
//! sudo wrappers ops uses (`apostille-sqlite` lists the rows,
//! `apostille-config get` reads each one). Plain values land in
//! `<target>.pulled.yaml` beside the setup file (0600; refused unless
//! git ignores it); secret rows come back by name only.
//!
//! `--secrets` (with a reason) also reads each secret through
//! `apostille-config reveal`, which audits every key on the server, for
//! backup and rebuild. Those land sops-encrypted to the operator's age
//! key in `<target>.pulled.sops.yaml`; plaintext secrets never touch
//! disk here. The wizard never asks for secrets. Either way, the result
//! lists every key where the server and the setup file disagree.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use serde::Serialize;
use serde_json::Value;

use super::file::{self, SetupFile, TargetKind};
use super::probe;

/// POSIX sh, run as hal after a `REASON_B64='…'` line (empty: no
/// secrets): one `value <key> <type> <base64>`, `secret <key>` or
/// `secretvalue <key> <base64>` line per config row. A failed read
/// stops the pull (the `&& echo x` keeps both the exit status and
/// the value's own trailing newlines).
const SCRIPT: &str = r#"set -u
S="sudo -n -u andrii-ai /usr/local/sbin"
read_row() {
    raw=$(printf '%s\n%s\n%s\n' "$1" "$2" "$3" | $S/apostille-config && echo x) || {
        echo "apostille-config $1 $2 failed" >&2; exit 1; }
    printf '%s' "${raw%x}" | base64 | tr -d '\n'
}
rows=$(echo "SELECT key || ' ' || type FROM config ORDER BY key;" | $S/apostille-sqlite) || {
    echo "apostille-sqlite failed (does this user have the apostille sudo rules?)" >&2; exit 1; }
printf '%s\n' "$rows" | while read -r key type; do
    case "$key" in ""|*[!a-z0-9._-]*) continue ;; esac
    if [ "$type" = secret ]; then
        if [ -z "$REASON_B64" ]; then echo "secret $key"; continue; fi
        reason=$(printf '%s' "$REASON_B64" | base64 -d)
        value=$(read_row reveal "$key" "$reason") || exit 1
        echo "secretvalue $key $value"
        continue
    fi
    value=$(read_row get "$key" "") || exit 1
    echo "value $key $type $value"
done
"#;

/// The script with its reason line (base64, so no reason text ever
/// reaches a shell as code).
fn script(reason: Option<&str>) -> String {
    let b64 = reason
        .map(|r| base64::engine::general_purpose::STANDARD.encode(r))
        .unwrap_or_default();
    format!("REASON_B64='{b64}'\n{SCRIPT}")
}

/// What one pull read.
#[derive(Clone, Default, Serialize, PartialEq)]
pub struct Pulled {
    pub target: String,
    pub host: String,
    /// RFC 3339 UTC.
    pub pulled_at: String,
    /// Plain values (text and bool rows).
    pub values: BTreeMap<String, Value>,
    /// Secret rows that are set.
    pub secrets_set: Vec<String>,
    /// Their values, on `--secrets` only (written sops-encrypted).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub secrets: BTreeMap<String, String>,
}

/// Secret values never reach a log or a panic message.
impl std::fmt::Debug for Pulled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pulled")
            .field("target", &self.target)
            .field("host", &self.host)
            .field("pulled_at", &self.pulled_at)
            .field("values", &self.values)
            .field("secrets_set", &self.secrets_set)
            .field("secrets", &format!("({} redacted)", self.secrets.len()))
            .finish()
    }
}

/// One key where the server and the setup file disagree.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Diff {
    pub key: String,
    /// What the setup file gives the target (`None`: not set there).
    pub file: Option<String>,
    /// What the server holds (`None`: no row).
    pub live: Option<String>,
}

/// `<setup dir>/<target>.pulled.yaml`, or `.pulled.sops.yaml` with
/// secrets.
pub fn path_for(setup: &Path, target: &str, secrets: bool) -> PathBuf {
    let name = if secrets {
        format!("{target}.pulled.sops.yaml")
    } else {
        format!("{target}.pulled.yaml")
    };
    setup.parent().unwrap_or_else(|| Path::new(".")).join(name)
}

/// The server target a bare `setup pull` reads: `production`, else the
/// only server target.
pub fn default_target(f: &SetupFile) -> Option<String> {
    let servers: Vec<&String> = f
        .targets
        .iter()
        .filter(|(_, t)| t.kind == TargetKind::Server)
        .map(|(name, _)| name)
        .collect();
    servers
        .iter()
        .find(|n| n.as_str() == "production")
        .or(servers.first())
        .map(|n| n.to_string())
}

/// A `--secrets` reason: one line, 3–200 characters (the server checks
/// the same and audits it with each key).
pub fn reason_ok(reason: &str) -> Result<String, String> {
    let line = reason.trim();
    if line.contains(['\n', '\r']) || !(3..=200).contains(&line.chars().count()) {
        return Err("--secrets needs --reason \"<who, why>\" (one line, 3-200 characters)".into());
    }
    Ok(line.to_string())
}

/// Read [`SCRIPT`]'s output. Remote text is untrusted: keys must look
/// like registry keys and each value is clipped.
pub fn parse(target: &str, host: &str, at: &str, text: &str) -> Pulled {
    let mut pulled = Pulled {
        target: target.to_string(),
        host: host.to_string(),
        pulled_at: at.to_string(),
        ..Pulled::default()
    };
    let key_ok = |k: &str| {
        !k.is_empty()
            && k.len() <= 100
            && k.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
    };
    // `config-get` and `config-reveal` print the value and a newline.
    let decode = |b64: &str| {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        let text = text.strip_suffix('\n').unwrap_or(&text);
        text.chars().take(10_000).collect::<String>()
    };
    for line in text.lines() {
        let mut words = line.split(' ');
        match (words.next(), words.next()) {
            (Some("secret"), Some(key)) if key_ok(key) => pulled.secrets_set.push(key.to_string()),
            (Some("secretvalue"), Some(key)) if key_ok(key) => {
                pulled.secrets_set.push(key.to_string());
                let value = decode(words.next().unwrap_or_default());
                pulled.secrets.insert(key.to_string(), value);
            }
            (Some("value"), Some(key)) if key_ok(key) => {
                let kind = words.next().unwrap_or_default();
                let text = decode(words.next().unwrap_or_default());
                let value = match kind {
                    // The store keeps bools as `1`/`0`.
                    "bool" => Value::Bool(matches!(text.trim(), "1" | "true")),
                    _ => Value::String(text),
                };
                pulled.values.insert(key.to_string(), value);
            }
            _ => {}
        }
    }
    pulled
}

fn shown(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Every key where `pulled` and the setup file's view of the target
/// disagree. Secrets compare by presence, and by value when revealed;
/// a secret's value is never shown.
pub fn diff(f: &SetupFile, pulled: &Pulled) -> Result<Vec<Diff>, String> {
    let want = f.target_config(&pulled.target)?;
    let mut keys: Vec<&String> = want
        .keys()
        .chain(pulled.values.keys())
        .chain(pulled.secrets_set.iter())
        .collect();
    keys.sort();
    keys.dedup();
    let mut out = Vec::new();
    for key in keys {
        let (file, live) = if file::is_secret(key) {
            let set = |yes: bool| yes.then(|| "(secret, set)".to_string());
            let other = match (want.get(key), pulled.secrets.get(key)) {
                (Some(mine), Some(theirs)) => shown(mine) != *theirs,
                _ => false,
            };
            let live = if other {
                Some("(secret, different value)".to_string())
            } else {
                set(pulled.secrets_set.contains(key))
            };
            (set(want.contains_key(key)), live)
        } else {
            (want.get(key).map(shown), pulled.values.get(key).map(shown))
        };
        if file != live {
            out.push(Diff {
                key: key.clone(),
                file,
                live,
            });
        }
    }
    Ok(out)
}

/// The pulled file's text: a header, then [`Pulled`] as YAML.
pub fn render(pulled: &Pulled) -> Result<String, String> {
    let yaml = serde_yaml_ng::to_string(pulled).map_err(|e| e.to_string())?;
    let what = if pulled.secrets.is_empty() {
        "# Plain values only: secret rows are listed by name (pull --secrets\n\
         # reads them into the sops-encrypted .pulled.sops.yaml)."
    } else {
        "# Secrets included: this file is sops-encrypted to the operator's\n\
         # age key (read it with `sops decrypt`)."
    };
    Ok(format!(
        "# What {} ({}) holds, pulled by `cargo xtask setup pull` at {}.\n\
         {what}\n# Git-ignored; never commit it.\n{yaml}",
        pulled.target, pulled.host, pulled.pulled_at
    ))
}

fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut out = options
        .open(path)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    out.write_all(text.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))
}

/// What to say when the server cannot reveal yet.
fn explain(host: &str, error: String) -> String {
    if error.contains("op must be set|get|verify\n") || error.ends_with("op must be set|get|verify")
    {
        format!(
            "{error}\n{host}'s apostille-config wrapper predates reveal: deploy this build, then re-run `cargo xtask provision` on {host} to install the wrapper"
        )
    } else if error.contains("unknown task: config-reveal") {
        format!("{error}\n{host} runs an xtask without config-reveal: deploy this build first")
    } else {
        error
    }
}

/// Pull `target`'s values (and, with a `reason`, its secrets), write
/// them beside `setup`, and diff them against the setup file.
pub fn run(
    f: &SetupFile,
    setup: &Path,
    target: &str,
    at: &str,
    reason: Option<&str>,
) -> Result<(PathBuf, Pulled, Vec<Diff>), String> {
    let t = f
        .targets
        .get(target)
        .ok_or_else(|| format!("no target {target:?}"))?;
    if t.kind != TargetKind::Server {
        return Err(format!(
            "{target} is a {:?} target; pull reads server targets only",
            t.kind
        ));
    }
    let host = t
        .ssh
        .clone()
        .ok_or_else(|| format!("targets.{target}: needs an ssh alias"))?;
    let reason = reason.map(reason_ok).transpose()?;
    if reason.is_some() {
        if file::is_plain(setup) {
            return Err(format!(
                "--secrets writes sops-encrypted beside the sops setup file ({}); {} is plain",
                file::DEFAULT_PATH,
                setup.display()
            ));
        }
        if let Some(why) = file::sops_missing() {
            return Err(format!("--secrets needs sops and an age key: {why}"));
        }
    }
    let path = path_for(setup, target, reason.is_some());
    file::guard_path(&path)?;
    let text = probe::ssh_script(&host, &script(reason.as_deref()), Duration::from_secs(300))
        .map_err(|e| explain(&host, e))?;
    let pulled = parse(target, &host, at, &text);
    if pulled.values.is_empty() && pulled.secrets_set.is_empty() {
        return Err(format!("ssh {host}: no config rows came back"));
    }
    let text = render(&pulled)?;
    if reason.is_some() {
        if pulled.secrets.len() != pulled.secrets_set.len() {
            return Err(format!(
                "ssh {host}: {} of {} secrets came back; nothing written",
                pulled.secrets.len(),
                pulled.secrets_set.len()
            ));
        }
        file::save_sops_yaml(&path, &text)?;
    } else {
        write_private(&path, &text)?;
    }
    let diffs = diff(f, &pulled)?;
    Ok((path, pulled, diffs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    #[test]
    fn parses_rows_and_drops_anything_odd() {
        let text = format!(
            "value auth.rp_id text {}\nvalue acme.enabled bool {}\nvalue quic.enabled bool {}\n\
             secret meta.api_key\nvalue Bad;Key text {}\nnoise\nsecret ../x\n",
            b64("admin.andrii.ai\n"),
            b64("1\n"),
            b64("0\n"),
            b64("x\n"),
        );
        let p = parse("production", "andrii.ai", "2026-11-04T00:00:00Z", &text);
        assert_eq!(p.values["auth.rp_id"], "admin.andrii.ai");
        assert_eq!(
            p.values["acme.enabled"], true,
            "the store keeps bools as 1/0"
        );
        assert_eq!(p.values["quic.enabled"], false);
        assert_eq!(p.values.len(), 3);
        assert_eq!(p.secrets_set, vec!["meta.api_key".to_string()]);
        assert!(p.secrets.is_empty());
        let yaml = render(&p).unwrap();
        assert!(yaml.starts_with("# What production (andrii.ai)"), "{yaml}");
        assert!(yaml.contains("auth.rp_id: admin.andrii.ai"), "{yaml}");
        assert!(!yaml.contains("\nsecrets:"), "{yaml}");
    }

    #[test]
    fn diffs_name_what_the_server_and_the_file_disagree_on() {
        let f = SetupFile::template();
        let mut p = Pulled {
            target: "production".into(),
            ..Pulled::default()
        };
        p.values
            .insert("auth.rp_id".into(), Value::String("admin.andrii.ai".into()));
        p.values.insert(
            "auth.origin".into(),
            Value::String("https://andrii.ai".into()),
        );
        p.secrets_set.push("meta.api_key".into());
        let diffs = diff(&f, &p).unwrap();
        let rp = diffs.iter().find(|d| d.key == "auth.rp_id").unwrap();
        assert_eq!(rp.file.as_deref(), Some("andrii.ai"));
        assert_eq!(rp.live.as_deref(), Some("admin.andrii.ai"));
        assert!(
            !diffs.iter().any(|d| d.key == "auth.origin"),
            "equal values"
        );
        let key = diffs.iter().find(|d| d.key == "meta.api_key").unwrap();
        assert_eq!(key.file, None);
        assert_eq!(key.live.as_deref(), Some("(secret, set)"));
    }

    #[test]
    fn pulls_land_beside_the_setup_file_and_read_servers_only() {
        assert_eq!(
            path_for(Path::new("private/config.yaml"), "production", false),
            Path::new("private/production.pulled.yaml")
        );
        assert_eq!(
            path_for(Path::new("private/config.yaml"), "production", true),
            Path::new("private/production.pulled.sops.yaml")
        );
        let f = SetupFile::template();
        assert_eq!(default_target(&f).as_deref(), Some("production"));
        let err = run(&f, Path::new("private/config.yaml"), "staging", "now", None).unwrap_err();
        assert!(err.contains("server targets only"), "{err}");
        // Secrets never land in the plain local file.
        let plain = crate::repo_root().join(file::LOCAL_PATH);
        let err = run(&f, &plain, "production", "now", Some("BDR drill")).unwrap_err();
        assert!(err.contains("sops-encrypted"), "{err}");
        let err = run(
            &f,
            Path::new("private/config.yaml"),
            "production",
            "now",
            Some("x"),
        )
        .unwrap_err();
        assert!(err.contains("--reason"), "{err}");
    }

    #[test]
    fn secrets_are_read_with_a_reason_and_never_shown() {
        for bad in ["", "no", "two\nlines", &"x".repeat(201)] {
            assert!(reason_ok(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            reason_ok("  BDR drill, Andrii ").unwrap(),
            "BDR drill, Andrii"
        );
        // The reason reaches the remote shell as base64 only.
        let sh = script(Some("x'; rm -rf / #"));
        assert!(!sh.contains("rm -rf"), "{sh}");
        assert!(script(None).starts_with("REASON_B64=''\n"));
        let text = format!(
            "secretvalue meta.api_key {}\nsecretvalue apple.private_key {}\n",
            b64("LLM_1_synthetic\n"),
            b64("-----BEGIN SYNTHETIC-----\nabc\n-----END SYNTHETIC-----\n\n"),
        );
        let p = parse("production", "andrii.ai", "now", &text);
        assert_eq!(p.secrets["meta.api_key"], "LLM_1_synthetic");
        assert!(p.secrets["apple.private_key"].ends_with("-----END SYNTHETIC-----\n"));
        assert_eq!(p.secrets_set.len(), 2);
        assert!(!format!("{p:?}").contains("LLM_1"), "Debug redacts");
        assert!(render(&p).unwrap().contains("sops-encrypted"));
        // Diffs say a secret differs, never what it is.
        let mut f = SetupFile::template();
        f.targets
            .get_mut("production")
            .unwrap()
            .config
            .insert("meta.api_key".into(), Value::String("LLM_1_older".into()));
        let d = diff(&f, &p).unwrap();
        let key = d.iter().find(|d| d.key == "meta.api_key").unwrap();
        assert_eq!(key.live.as_deref(), Some("(secret, different value)"));
        assert!(!format!("{d:?}").contains("LLM_1"));
        assert!(
            explain(
                "andrii.ai",
                "ABORT: op must be set|get|verify\nfailed".into()
            )
            .contains("cargo xtask provision")
        );
        assert_eq!(explain("h", "other".into()), "other");
    }
}
