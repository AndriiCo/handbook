//! Rust crates from `cargo metadata` (Cargo.lock), filtered to the
//! production target, scoped by how the workspace reaches them.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use super::{Package, Scope, scan_dir};

const TARGET: &str = "x86_64-unknown-linux-gnu";

pub(crate) fn collect(root: &Path) -> Result<Vec<Package>, String> {
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--format-version", "1"])
        .args(["--filter-platform", TARGET])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let meta: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("cargo metadata output: {e}"))?;
    from_metadata(&meta, true)
}

/// Rows from parsed metadata. `scan` reads licence files from each
/// crate's source dir (off in unit tests, which have none).
pub(crate) fn from_metadata(meta: &Value, scan: bool) -> Result<Vec<Package>, String> {
    let empty = Vec::new();
    let packages: BTreeMap<&str, &Value> = meta["packages"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p)))
        .collect();
    let members: BTreeSet<&str> = meta["workspace_members"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let scopes = scopes(meta, &packages, &members)?;

    let mut out = Vec::new();
    for (id, pkg) in &packages {
        if members.contains(id) {
            continue;
        }
        let Some(tags) = scopes.get(id) else {
            continue; // not reached on the production target
        };
        let name = pkg["name"].as_str().unwrap_or_default().to_string();
        let version = pkg["version"].as_str().unwrap_or_default().to_string();
        let url = ["repository", "homepage", "documentation"]
            .iter()
            .find_map(|k| pkg[*k].as_str().filter(|s| !s.is_empty()))
            .map(|s| s.trim_end_matches('/').trim_end_matches(".git").to_string())
            .unwrap_or_else(|| format!("https://crates.io/crates/{name}/{version}"));
        let authors = pkg["authors"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .filter_map(Value::as_str)
            .map(strip_email)
            .collect();
        let files = match pkg["manifest_path"].as_str().map(Path::new) {
            Some(manifest) if scan => scan_dir(manifest.parent().unwrap_or(manifest)),
            _ => Vec::new(),
        };
        out.push(Package {
            ecosystem: "crates.io".into(),
            name,
            version,
            url,
            license: pkg["license"].as_str().unwrap_or_default().to_string(),
            scopes: tags.clone(),
            copyrights: Vec::new(),
            authors,
            files,
            note: None,
        });
    }
    Ok(out)
}

/// `Name <mail>` → `Name`.
fn strip_email(author: &str) -> String {
    author
        .split('<')
        .next()
        .unwrap_or(author)
        .trim()
        .to_string()
}

/// Scope tags per package id: normal deps of `web` are app, of
/// `xtask` worker; anything under a build-dependency, or a proc-macro
/// (it runs inside rustc), is build; a workspace dev-dependency is test.
fn scopes<'a>(
    meta: &'a Value,
    packages: &BTreeMap<&'a str, &'a Value>,
    members: &BTreeSet<&'a str>,
) -> Result<BTreeMap<&'a str, BTreeSet<Scope>>, String> {
    let mut edges: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for node in meta["resolve"]["nodes"].as_array().into_iter().flatten() {
        let Some(id) = node["id"].as_str() else {
            continue;
        };
        let list = edges.entry(id).or_default();
        for dep in node["deps"].as_array().into_iter().flatten() {
            let Some(pkg) = dep["pkg"].as_str() else {
                continue;
            };
            for kind in dep["dep_kinds"].as_array().into_iter().flatten() {
                list.push((pkg, kind["kind"].as_str().unwrap_or("normal")));
            }
        }
    }
    let member_named = |name: &str| {
        members
            .iter()
            .copied()
            .find(|id| packages.get(id).is_some_and(|p| p["name"] == name))
            .ok_or_else(|| format!("cargo metadata: no workspace member {name:?}"))
    };
    let is_proc_macro = |id: &str| {
        packages.get(id).is_some_and(|p| {
            p["targets"].as_array().is_some_and(|ts| {
                ts.iter().any(|t| {
                    t["kind"]
                        .as_array()
                        .is_some_and(|k| k.iter().any(|k| k == "proc-macro"))
                })
            })
        })
    };

    let mut queue: VecDeque<(&str, Scope)> = VecDeque::new();
    queue.push_back((member_named("web")?, Scope::App));
    queue.push_back((member_named("xtask")?, Scope::Worker));
    for member in members {
        for (dep, kind) in edges.get(member).into_iter().flatten() {
            if *kind == "dev" {
                queue.push_back((dep, Scope::Test));
            }
        }
    }
    let mut seen: BTreeSet<(&str, Scope)> = BTreeSet::new();
    let mut out: BTreeMap<&str, BTreeSet<Scope>> = BTreeMap::new();
    while let Some((id, scope)) = queue.pop_front() {
        let scope = if is_proc_macro(id) && matches!(scope, Scope::App | Scope::Worker) {
            Scope::Build
        } else {
            scope
        };
        if !seen.insert((id, scope)) {
            continue;
        }
        out.entry(id).or_default().insert(scope);
        for (dep, kind) in edges.get(id).into_iter().flatten() {
            let next = match *kind {
                "dev" => continue, // seeded above, for workspace members only
                "build" if scope != Scope::Test => Scope::Build,
                _ => scope,
            };
            queue.push_back((dep, next));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, deps: &[(&str, &str)]) -> Value {
        let deps: Vec<Value> = deps
            .iter()
            .map(|(pkg, kind)| {
                let kind = if *kind == "normal" {
                    Value::Null
                } else {
                    Value::from(*kind)
                };
                serde_json::json!({"pkg": pkg, "dep_kinds": [{"kind": kind}]})
            })
            .collect();
        serde_json::json!({"id": id, "deps": deps})
    }

    fn pkg(id: &str, kind: &str) -> Value {
        serde_json::json!({
            "id": id, "name": id, "version": "1.0.0", "license": "MIT",
            "repository": format!("https://github.com/x/{id}.git"),
            "authors": [format!("{id} dev <{id}@example.test>")],
            "targets": [{"kind": [kind]}],
        })
    }

    #[test]
    fn scopes_follow_dependency_kinds() {
        let meta = serde_json::json!({
            "workspace_members": ["web", "xtask", "lib"],
            "packages": [
                pkg("web", "bin"), pkg("xtask", "bin"), pkg("lib", "lib"),
                pkg("serde", "lib"), pkg("derive", "proc-macro"), pkg("cc", "lib"),
                pkg("ureq", "lib"), pkg("tempfile", "lib"), pkg("unused", "lib"),
            ],
            "resolve": {"nodes": [
                node("web", &[("lib", "normal"), ("tempfile", "dev")]),
                node("xtask", &[("lib", "normal"), ("ureq", "normal")]),
                node("lib", &[("serde", "normal"), ("cc", "build")]),
                node("serde", &[("derive", "normal")]),
                node("derive", &[]), node("cc", &[]), node("ureq", &[]),
                node("tempfile", &[]), node("unused", &[]),
            ]},
        });
        let rows = from_metadata(&meta, false).unwrap();
        let tags = |name: &str| {
            rows.iter()
                .find(|p| p.name == name)
                .map(|p| p.scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        };
        assert_eq!(tags("serde"), Some(vec!["app", "worker"]));
        assert_eq!(tags("derive"), Some(vec!["build"]));
        assert_eq!(tags("cc"), Some(vec!["build"]));
        assert_eq!(tags("ureq"), Some(vec!["worker"]));
        assert_eq!(tags("tempfile"), Some(vec!["test"]));
        assert_eq!(tags("unused"), None);
        assert_eq!(tags("lib"), None, "workspace members are not rows");
        let serde = rows.iter().find(|p| p.name == "serde").unwrap();
        assert_eq!(serde.url, "https://github.com/x/serde");
        assert_eq!(serde.authors, vec!["serde dev"]);
    }
}
