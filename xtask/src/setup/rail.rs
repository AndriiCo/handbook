//! The schema and migration rail (docs/ops/deploy.md § Rails): a
//! release whose schema constants, DDL or migration files differ from
//! the live release ships only with `--migration-ack <path>`, a backup
//! of `handbook.sqlite` taken on the server in the last 2 hours. No
//! older binary opens a migrated store, so that backup is the rollback.

use std::path::Path;
use std::process::Command;

/// Directory a migration acknowledgement must point into.
const ACK_DIR: &str = "/var/lib/apostille/";

/// Whether `path` is a plain path under [`ACK_DIR`] (safe to quote in
/// a remote shell: no `..`, no quotes, no spaces).
pub fn valid_ack_path(path: &str) -> bool {
    path.strip_prefix(ACK_DIR).is_some_and(|rest| {
        !rest.is_empty()
            && !rest.split('/').any(|seg| seg == "..")
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b))
    })
}

/// The version from a `/healthz` body (`"version":"0.1.0+<sha>"`).
pub fn live_sha(body: &str) -> Option<String> {
    let rest = body.split("\"version\":\"").nth(1)?;
    let version = rest.split('"').next()?;
    let sha = version.split_once('+')?.1;
    (sha.len() >= 7 && sha.len() <= 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| sha.to_string())
}

/// Lines of a `git diff` that change the schema: the version constants,
/// or added `ALTER`/`DROP`/`RENAME` statements.
pub fn schema_hits(diff: &str) -> Vec<String> {
    let ddl =
        regex::Regex::new(r"^\+.*\b(ALTER|DROP) (TABLE|INDEX|COLUMN)|^\+.*\bRENAME (TO|COLUMN)\b")
            .expect("static regex");
    diff.lines()
        .filter(|l| !l.starts_with("+++") && !l.starts_with("---"))
        .filter(|l| {
            let body = l.get(1..).unwrap_or_default().trim_start();
            ((l.starts_with('+') || l.starts_with('-'))
                && (body.starts_with("pub const SCHEMA_VERSION")
                    || body.starts_with("pub const P2_SCHEMA_VERSION")))
                || ddl.is_match(l)
        })
        .map(str::to_string)
        .collect()
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// What makes `sha` a migration relative to `live`, or `None`.
pub fn delta(repo: &Path, live: &str, sha: &str) -> Result<Option<String>, String> {
    git(repo, &["cat-file", "-e", &format!("{live}^{{commit}}")])
        .map_err(|_| format!("live SHA {live} is not in this repo (fetch first)"))?;
    let hits = schema_hits(&git(repo, &["diff", live, sha, "--", "crates", "xtask"])?);
    let migrations = git(
        repo,
        &[
            "diff",
            "--stat",
            live,
            sha,
            "--",
            "crates/handbook-store/migrations",
        ],
    )?;
    Ok(if !hits.is_empty() {
        Some(format!("schema delta vs live {live}:\n{}", hits.join("\n")))
    } else if !migrations.trim().is_empty() {
        Some(format!("migrations differ from live {live}"))
    } else {
        None
    })
}

/// Remote check that the acknowledged backup is non-empty and under
/// 2 hours old (exit 0 only then).
pub fn fresh_backup_script(path: &str) -> String {
    format!("test -s '{path}' && find '{path}' -mmin -120 | grep -q .")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_paths_stay_under_the_store() {
        assert!(valid_ack_path("/var/lib/apostille/backups/pre-013.sqlite"));
        for bad in [
            "/var/lib/apostille/",
            "/tmp/x.sqlite",
            "/var/lib/apostille/../etc/shadow",
            "/var/lib/apostille/a b",
            "/var/lib/apostille/x';rm -rf /;'",
        ] {
            assert!(!valid_ack_path(bad), "{bad}");
        }
    }

    #[test]
    fn reads_the_live_sha_from_healthz() {
        let body = r#"{"status":"ok","version":"0.1.0+56f0fbb","schema":13}"#;
        assert_eq!(live_sha(body).as_deref(), Some("56f0fbb"));
        assert_eq!(live_sha(r#"{"version":"0.1.0"}"#), None);
        assert_eq!(live_sha(r#"{"version":"0.1.0+zz"}"#), None);
    }

    #[test]
    fn finds_schema_changes_in_a_diff() {
        let diff = "\
+++ b/crates/handbook-store/src/db.rs
-pub const SCHEMA_VERSION: u32 = 12;
+pub const SCHEMA_VERSION: u32 = 13;
+    conn.execute(\"ALTER TABLE orders ADD COLUMN x\")?;
+// a comment that drops nothing
 unchanged ALTER TABLE context
";
        let hits = schema_hits(diff);
        assert_eq!(hits.len(), 3, "{hits:?}");
        assert!(schema_hits("+let x = 1;\n-let y = 2;\n").is_empty());
    }
}
