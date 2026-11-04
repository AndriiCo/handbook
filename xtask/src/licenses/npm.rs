//! npm packages from `web/package-lock.json` (lockfile v3), resolved the
//! way Node does (nearest `node_modules` up the tree), filtered to the
//! platforms that build the web apps.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use serde_json::Value;

use super::{Package, Scope, scan_dir};

/// devDependencies that exist only for the test suite.
const TEST_ROOTS: &[&str] = &["@playwright/test", "@axe-core/playwright"];

/// (os, cpu, libc) of the machines that run `npm ci`: the release Mac
/// and the Linux hosts (plant, the judge image).
const PLATFORMS: &[(&str, &str, Option<&str>)] =
    &[("darwin", "arm64", None), ("linux", "x64", Some("glibc"))];

pub(crate) fn collect(root: &Path) -> Result<Vec<Package>, String> {
    let web = root.join("web");
    let lock_path = web.join("package-lock.json");
    let raw = std::fs::read_to_string(&lock_path)
        .map_err(|e| format!("read {}: {e}", lock_path.display()))?;
    let lock: Value =
        serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", lock_path.display()))?;
    if !web.join("node_modules").is_dir() {
        return Err("web/node_modules is missing: run `npm ci --prefix web` first".to_string());
    }
    from_lock(&lock, Some(&web))
}

/// Rows from a parsed lockfile; `web` (when set) is where installed
/// packages are read for repository, author and licence files.
pub(crate) fn from_lock(lock: &Value, web: Option<&Path>) -> Result<Vec<Package>, String> {
    let Some(packages) = lock["packages"].as_object() else {
        return Err("package-lock.json has no `packages` (lockfile v2+ needed)".to_string());
    };
    let root = &packages[""];
    let mut queue: VecDeque<(String, Scope)> = VecDeque::new();
    let deps = |v: &Value| -> Vec<String> {
        v.as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default()
    };
    for name in deps(&root["dependencies"]) {
        queue.push_back((resolve(packages, "", &name).unwrap_or_default(), Scope::App));
    }
    for name in deps(&root["devDependencies"]) {
        let scope = if TEST_ROOTS.contains(&name.as_str()) {
            Scope::Test
        } else {
            Scope::Build
        };
        queue.push_back((resolve(packages, "", &name).unwrap_or_default(), scope));
    }

    let mut scopes: BTreeMap<String, BTreeSet<Scope>> = BTreeMap::new();
    let mut seen: BTreeSet<(String, Scope)> = BTreeSet::new();
    while let Some((path, scope)) = queue.pop_front() {
        let Some(entry) = packages.get(&path) else {
            continue;
        };
        if path.is_empty() || !on_build_platform(entry) || !seen.insert((path.clone(), scope)) {
            continue;
        }
        scopes.entry(path.clone()).or_default().insert(scope);
        for field in ["dependencies", "optionalDependencies", "peerDependencies"] {
            for name in deps(&entry[field]) {
                if let Some(dep) = resolve(packages, &path, &name) {
                    queue.push_back((dep, scope));
                }
            }
        }
    }

    // One row per name@version, however many paths install it.
    let mut rows: BTreeMap<(String, String), Package> = BTreeMap::new();
    for (path, tags) in scopes {
        let entry = &packages[&path];
        let name = entry["name"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| package_name(&path).to_string());
        let version = entry["version"].as_str().unwrap_or_default().to_string();
        let installed = web
            .map(|w| w.join(&path))
            .filter(|dir| dir.join("package.json").is_file());
        let manifest: Value = installed
            .as_ref()
            .and_then(|dir| std::fs::read_to_string(dir.join("package.json")).ok())
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or(Value::Null);
        let license = entry["license"]
            .as_str()
            .or_else(|| manifest["license"].as_str())
            .unwrap_or_default()
            .to_string();
        let url = repository_url(&manifest["repository"])
            .or_else(|| manifest["homepage"].as_str().map(str::to_string))
            .unwrap_or_else(|| format!("https://www.npmjs.com/package/{name}/v/{version}"));
        let authors = match &manifest["author"] {
            Value::String(s) => vec![s.split('<').next().unwrap_or(s).trim().to_string()],
            Value::Object(o) => o
                .get("name")
                .and_then(Value::as_str)
                .map(|s| vec![s.to_string()])
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let row = rows
            .entry((name.clone(), version.clone()))
            .or_insert_with(|| Package {
                ecosystem: "npm".into(),
                name,
                version,
                url,
                license,
                scopes: BTreeSet::new(),
                copyrights: Vec::new(),
                authors,
                files: installed.as_deref().map(scan_dir).unwrap_or_default(),
                note: None,
            });
        row.scopes.extend(tags);
    }
    Ok(rows.into_values().collect())
}

/// `node_modules/a/node_modules/@s/b` → `@s/b`.
fn package_name(path: &str) -> &str {
    path.rsplit_once("node_modules/").map_or(path, |(_, n)| n)
}

/// Node's lookup: `<from>/node_modules/<name>`, then each ancestor's.
fn resolve(packages: &serde_json::Map<String, Value>, from: &str, name: &str) -> Option<String> {
    let mut base = from.to_string();
    loop {
        let candidate = if base.is_empty() {
            format!("node_modules/{name}")
        } else {
            format!("{base}/node_modules/{name}")
        };
        if packages.contains_key(&candidate) {
            return Some(candidate);
        }
        if base.is_empty() {
            return None;
        }
        base = base
            .rfind("/node_modules/")
            .map_or(String::new(), |i| base[..i].to_string());
    }
}

/// True when an `os`/`cpu`/`libc`-restricted package installs on one of
/// [`PLATFORMS`] (unrestricted packages always do).
fn on_build_platform(entry: &Value) -> bool {
    let allows = |field: &str, want: &str| -> bool {
        let Some(list) = entry[field].as_array() else {
            return true;
        };
        let items: Vec<&str> = list.iter().filter_map(Value::as_str).collect();
        if items.iter().any(|i| *i == format!("!{want}")) {
            return false;
        }
        items.iter().all(|i| i.starts_with('!')) || items.contains(&want)
    };
    PLATFORMS.iter().any(|(os, cpu, libc)| {
        allows("os", os) && allows("cpu", cpu) && libc.is_none_or(|l| allows("libc", l))
    })
}

/// A browsable URL for a package.json `repository` (string shorthand
/// or `{url, directory}`).
fn repository_url(repo: &Value) -> Option<String> {
    let (raw, directory) = match repo {
        Value::String(s) => (s.as_str(), None),
        Value::Object(o) => (
            o.get("url")?.as_str()?,
            o.get("directory").and_then(Value::as_str),
        ),
        _ => return None,
    };
    let mut url = raw.trim().to_string();
    for (from, to) in [
        ("git+https://", "https://"),
        ("git+ssh://git@", "https://"),
        ("git://", "https://"),
        ("ssh://git@", "https://"),
        ("git@github.com:", "https://github.com/"),
        ("github:", "https://github.com/"),
        ("gitlab:", "https://gitlab.com/"),
        ("bitbucket:", "https://bitbucket.org/"),
    ] {
        if let Some(rest) = url.strip_prefix(from) {
            url = format!("{to}{rest}");
        }
    }
    if !url.contains(':') && url.split('/').count() == 2 {
        url = format!("https://github.com/{url}"); // "user/repo" shorthand
    }
    let mut url = url
        .split('#')
        .next()
        .unwrap_or_default()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_string();
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return None;
    }
    if let Some(dir) = directory.filter(|d| !d.is_empty()) {
        url = format!("{url}/tree/HEAD/{}", dir.trim_matches('/'));
    }
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scopes_resolve_nested_paths_and_skip_foreign_platforms() {
        let lock = json!({"lockfileVersion": 3, "packages": {
            "": {"dependencies": {"clsx": "^2"}, "devDependencies": {"vite": "^8", "@playwright/test": "^1"}},
            "node_modules/clsx": {"version": "2.1.1", "license": "MIT"},
            "node_modules/vite": {"version": "8.3.5", "license": "MIT", "dev": true,
                "dependencies": {"picomatch": "^4"},
                "optionalDependencies": {"binding-win32": "1", "binding-darwin": "1"}},
            "node_modules/vite/node_modules/picomatch": {"version": "4.0.7", "license": "MIT"},
            "node_modules/picomatch": {"version": "2.3.1", "license": "MIT"},
            "node_modules/binding-win32": {"version": "1.0.0", "os": ["win32"], "cpu": ["x64"]},
            "node_modules/binding-darwin": {"version": "1.0.0", "os": ["darwin"], "cpu": ["arm64"]},
            "node_modules/@playwright/test": {"version": "1.64.1", "license": "Apache-2.0",
                "dependencies": {"clsx": "^2"}},
        }});
        let rows = from_lock(&lock, None).unwrap();
        let tags = |name: &str, version: &str| {
            rows.iter()
                .find(|p| p.name == name && p.version == version)
                .map(|p| p.scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        };
        assert_eq!(tags("clsx", "2.1.1"), Some(vec!["app", "test"]));
        assert_eq!(tags("picomatch", "4.0.7"), Some(vec!["build"]));
        assert_eq!(
            tags("picomatch", "2.3.1"),
            None,
            "shadowed by the nested copy"
        );
        assert_eq!(tags("binding-win32", "1.0.0"), None);
        assert_eq!(tags("binding-darwin", "1.0.0"), Some(vec!["build"]));
        assert_eq!(tags("@playwright/test", "1.64.1"), Some(vec!["test"]));
        let clsx = rows.iter().find(|p| p.name == "clsx").unwrap();
        assert_eq!(clsx.url, "https://www.npmjs.com/package/clsx/v/2.1.1");
    }

    #[test]
    fn repository_urls_normalise() {
        let cases = [
            (
                json!("git+https://github.com/a/b.git"),
                "https://github.com/a/b",
            ),
            (json!("github:a/b"), "https://github.com/a/b"),
            (json!("a/b"), "https://github.com/a/b"),
            (
                json!({"url": "git+ssh://git@github.com/a/mono.git", "directory": "packages/x"}),
                "https://github.com/a/mono/tree/HEAD/packages/x",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(repository_url(&input).as_deref(), Some(want));
        }
        assert_eq!(repository_url(&json!("file:../local")), None);
    }
}
