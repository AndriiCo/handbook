//! `cargo xtask licenses`: the software bill of materials page,
//! `crates/web/static/licenses.html`, served at andrii.ai/licenses.html.
//!
//! ```text
//! cargo xtask licenses                  # regenerate + commit if stale
//! cargo xtask licenses --force          # regenerate + commit now
//! cargo xtask licenses --check          # exit 1 if stale, write nothing
//! cargo xtask licenses --root <dir> --no-commit   # ship worktrees
//! ```
//!
//! Stale means the page is 7 days old or its inputs (lockfiles,
//! `docs/licenses/extra.toml`, toolchain pins, Dockerfiles, licence
//! texts) hash differently from when it was written. Both stamps live in
//! `<meta>` tags only, never in the visible page. `ci` runs the stale
//! check (and commits); both ship paths run it with `--no-commit` in
//! their throwaway worktree, so production never serves an old page.
//! docs/licenses/README.md has the details.

mod cargo;
mod extra;
mod npm;
mod render;
mod texts;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::flag;

/// Where the page lives, relative to the repo root.
pub(crate) const OUT: &str = "crates/web/static/licenses.html";
const TEXTS: &str = "docs/licenses/texts";
const MAX_AGE_SECS: i64 = 7 * 86_400;
/// Bump when the page layout changes, so every checkout regenerates.
const GENERATOR: &str = "licenses/1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Scope {
    App,
    Worker,
    Build,
    Test,
    Image,
}

impl Scope {
    pub(crate) const ALL: [Scope; 5] = [
        Scope::App,
        Scope::Worker,
        Scope::Build,
        Scope::Test,
        Scope::Image,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Scope::App => "app",
            Scope::Worker => "worker",
            Scope::Build => "build",
            Scope::Test => "test",
            Scope::Image => "image",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|scope| scope.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FileKind {
    License,
    Notice,
    Copyright,
}

#[derive(Debug, Clone)]
pub(crate) struct LicenseFile {
    pub(crate) kind: FileKind,
    pub(crate) text: String,
}

/// One BOM row before its licence files are matched.
#[derive(Debug, Clone)]
pub(crate) struct Package {
    pub(crate) ecosystem: String,
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) url: String,
    /// SPDX expression as declared ("" when the package declares none).
    pub(crate) license: String,
    pub(crate) scopes: BTreeSet<Scope>,
    pub(crate) copyrights: Vec<String>,
    pub(crate) authors: Vec<String>,
    pub(crate) files: Vec<LicenseFile>,
    pub(crate) note: Option<String>,
}

/// Licence, notice and copyright files at the top of a package dir
/// (plus a REUSE-style `LICENSES/` dir), sorted by name.
pub(crate) fn scan_dir(dir: &Path) -> Vec<LicenseFile> {
    const CODE: &[&str] = &[
        "rs", "js", "mjs", "cjs", "ts", "tsx", "json", "py", "html", "htm", "css", "map", "sh",
        "toml", "yml", "yaml", "png", "svg",
    ];
    let mut paths: Vec<(PathBuf, FileKind)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if path.is_dir() {
            if name == "licenses" {
                for inner in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                    if inner.path().is_file() {
                        paths.push((inner.path(), FileKind::License));
                    }
                }
            }
            continue;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if CODE.contains(&ext.as_str()) {
            continue;
        }
        let kind = if name.starts_with("notice") {
            FileKind::Notice
        } else if name.starts_with("copyright") {
            FileKind::Copyright
        } else if ["license", "licence", "copying", "unlicense", "unlicence"]
            .iter()
            .any(|p| name.starts_with(p))
        {
            FileKind::License
        } else {
            continue;
        };
        paths.push((path, kind));
    }
    paths.sort();
    paths
        .into_iter()
        .filter(|(p, _)| std::fs::metadata(p).is_ok_and(|m| m.len() <= 512 * 1024))
        .filter_map(|(p, kind)| {
            let bytes = std::fs::read(&p).ok()?;
            Some(LicenseFile {
                kind,
                text: String::from_utf8_lossy(&bytes).replace("\r\n", "\n"),
            })
        })
        .collect()
}

// ---- matched model --------------------------------------------------------

/// A BOM row after its licence files were matched.
pub(crate) struct Row {
    pub(crate) pkg: Package,
    /// Expression shown (declared, or detected from the files).
    pub(crate) expr: String,
    pub(crate) detected: bool,
    /// Licence id → anchor (canonical `license-<id>` or a variant).
    pub(crate) links: BTreeMap<String, String>,
    /// Variant texts whose licence the expression does not name.
    pub(crate) own_texts: Vec<String>,
    pub(crate) notices: Vec<String>,
    pub(crate) anchor: String,
}

pub(crate) struct Variant {
    pub(crate) primary: String,
    /// Holds most of `primary`'s canonical text (else: an extra file).
    pub(crate) close: bool,
    pub(crate) text: String,
    pub(crate) users: Vec<usize>,
}

pub(crate) struct Notice {
    pub(crate) text: String,
    pub(crate) users: Vec<usize>,
}

pub(crate) struct Bom {
    pub(crate) rows: Vec<Row>,
    /// Canonical licence id → rows linking to its canonical text.
    pub(crate) canonical: BTreeMap<String, Vec<usize>>,
    /// Anchor → variant text.
    pub(crate) variants: BTreeMap<String, Variant>,
    /// Anchor → NOTICE text.
    pub(crate) notices: BTreeMap<String, Notice>,
}

const ECOSYSTEM_ORDER: &[&str] = &[
    "crates.io",
    "npm",
    "toolchain",
    "docker",
    "apt",
    "pip",
    "binary",
    "model",
    "font",
];

fn sort_key(p: &Package) -> (usize, String, String) {
    let eco = ECOSYSTEM_ORDER
        .iter()
        .position(|e| *e == p.ecosystem)
        .unwrap_or(ECOSYSTEM_ORDER.len());
    (eco, p.name.to_lowercase(), p.version.clone())
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Match every package's files against the library and build the page model.
pub(crate) fn assemble(mut packages: Vec<Package>, lib: &texts::Library) -> Bom {
    packages.sort_by_key(sort_key);
    let mut bom = Bom {
        rows: Vec::new(),
        canonical: BTreeMap::new(),
        variants: BTreeMap::new(),
        notices: BTreeMap::new(),
    };
    for (idx, mut pkg) in packages.into_iter().enumerate() {
        let (licences, exceptions) = texts::ids(&pkg.license);
        let declared: BTreeSet<String> = licences.union(&exceptions).cloned().collect();
        let mut matched: BTreeSet<String> = BTreeSet::new();
        let mut links = BTreeMap::new();
        let mut own_texts = Vec::new();
        let mut notices = Vec::new();
        let mut copyrights = std::mem::take(&mut pkg.copyrights);
        for file in &pkg.files {
            copyrights.extend(texts::copyright_lines(&file.text));
            match file.kind {
                FileKind::License => match texts::match_file(lib, &file.text, &declared) {
                    texts::Match::Canonical(ids) => matched.extend(ids),
                    texts::Match::Variant { primary, close } => {
                        let anchor = format!(
                            "license-{}-{}",
                            slug(&primary),
                            texts::key(&texts::normalise(&file.text))
                        );
                        let variant = bom.variants.entry(anchor.clone()).or_insert(Variant {
                            primary: primary.clone(),
                            close,
                            text: texts::without_copyrights(&file.text),
                            users: Vec::new(),
                        });
                        if !variant.users.contains(&idx) {
                            variant.users.push(idx);
                        }
                        if close && licences.contains(&primary) && !links.contains_key(&primary) {
                            links.insert(primary, anchor);
                        } else if !own_texts.contains(&anchor) {
                            own_texts.push(anchor);
                        }
                    }
                    texts::Match::Pointer => {}
                },
                FileKind::Notice => {
                    let words = notice_words(&file.text);
                    if words.is_empty() {
                        continue;
                    }
                    let anchor = format!("notice-{}", texts::key(&words));
                    let notice = bom.notices.entry(anchor.clone()).or_insert(Notice {
                        text: file.text.trim().to_string(),
                        users: Vec::new(),
                    });
                    if !notice.users.contains(&idx) {
                        notice.users.push(idx);
                    }
                    if !notices.contains(&anchor) {
                        notices.push(anchor);
                    }
                }
                FileKind::Copyright => {}
            }
        }
        // "Copyright 2015 X." and "Copyright (c) 2015 X" are one notice.
        let mut seen = BTreeSet::new();
        copyrights.retain(|c| {
            let key: String = c
                .to_lowercase()
                .replace("(c)", "")
                .chars()
                .filter(|ch| ch.is_alphanumeric())
                .collect();
            seen.insert(key)
        });
        pkg.copyrights = copyrights;

        let detected = pkg.license.trim().is_empty() && !matched.is_empty();
        let expr = if detected {
            matched.iter().cloned().collect::<Vec<_>>().join(" AND ")
        } else {
            pkg.license.trim().to_string()
        };
        let (licences, exceptions) = texts::ids(&expr);
        for id in licences.iter().chain(exceptions.iter()) {
            if !links.contains_key(id) {
                links.insert(id.clone(), format!("license-{}", slug(id)));
                bom.canonical.entry(id.clone()).or_default().push(idx);
            }
        }
        let anchor = format!(
            "pkg-{}-{}-{}",
            slug(&pkg.ecosystem),
            slug(&pkg.name),
            slug(&pkg.version)
        );
        pkg.files.clear(); // matched; the texts live in the library and variants now
        bom.rows.push(Row {
            pkg,
            expr,
            detected,
            links,
            own_texts,
            notices,
            anchor,
        });
    }
    bom
}

/// Words of a NOTICE file (copyright lines kept: they are the notice).
fn notice_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

// ---- staleness --------------------------------------------------------------

/// sha256 over every input of the page, so a dependency change makes it
/// stale before the 7 days are up.
pub(crate) fn inputs_hash(root: &Path) -> Result<String, String> {
    let mut files: Vec<String> = [
        "Cargo.lock",
        "web/package-lock.json",
        extra::EXTRA,
        "rust-toolchain.toml",
        ".nvmrc",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    files.extend(extra::load(root)?.dockerfiles.into_keys());
    let mut texts: Vec<String> = std::fs::read_dir(root.join(TEXTS))
        .map_err(|e| format!("read {TEXTS}: {e}"))?
        .flatten()
        .map(|e| format!("{TEXTS}/{}", e.file_name().to_string_lossy()))
        .collect();
    texts.sort();
    files.extend(texts);
    let mut hasher = Sha256::new();
    hasher.update(GENERATOR.as_bytes());
    for file in files {
        hasher.update(file.as_bytes());
        hasher.update([0]);
        match std::fs::read(root.join(&file)) {
            Ok(bytes) => {
                hasher.update((bytes.len() as u64).to_le_bytes());
                hasher.update(&bytes);
            }
            Err(_) => hasher.update(b"missing"),
        }
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(format!("sha256:{hex}"))
}

/// Content of `<meta name="{name}" content="…">` in a page.
pub(crate) fn meta(html: &str, name: &str) -> Option<String> {
    let needle = format!("<meta name=\"{name}\" content=\"");
    let start = html.find(&needle)? + needle.len();
    let len = html[start..].find('"')?;
    Some(html[start..start + len].to_string())
}

/// `YYYY-MM-DDTHH:MM:SSZ` → unix seconds (the inverse of
/// `setup::brand::rfc3339`).
pub(crate) fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[19] != b'Z' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // days_from_civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    Forced,
    Missing,
    InputsChanged,
    Age,
}

impl Reason {
    fn as_str(self) -> &'static str {
        match self {
            Reason::Forced => "forced",
            Reason::Missing => "no page yet",
            Reason::InputsChanged => "inputs changed",
            Reason::Age => "7-day refresh",
        }
    }
}

/// Why the page at `existing` needs regenerating, if it does.
pub(crate) fn stale(existing: Option<&str>, inputs: &str, now: i64) -> Option<Reason> {
    let Some(html) = existing else {
        return Some(Reason::Missing);
    };
    if meta(html, "bom:inputs").as_deref() != Some(inputs) {
        return Some(Reason::InputsChanged);
    }
    match meta(html, "bom:generated").and_then(|g| parse_rfc3339(&g)) {
        Some(generated) if now - generated < MAX_AGE_SECS => None,
        _ => Some(Reason::Age),
    }
}

// ---- git --------------------------------------------------------------------

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Why the regenerated page must not be committed at `root`, if
/// anything: checked before the page is written.
fn commit_blocker(root: &Path) -> Option<String> {
    let Ok(top) = git(root, &["rev-parse", "--show-toplevel"]) else {
        return Some("not a git checkout".to_string());
    };
    let same = |a: &Path, b: &Path| a.canonicalize().ok() == b.canonicalize().ok();
    if !same(Path::new(&top), root) {
        return Some(format!(
            "{} is not the top of its checkout ({top})",
            root.display()
        ));
    }
    if git(root, &["symbolic-ref", "-q", "HEAD"]).is_err() {
        return Some("HEAD is detached".to_string());
    }
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
    ] {
        if let Ok(path) = git(root, &["rev-parse", "--git-path", marker])
            && root.join(&path).exists()
        {
            return Some(format!(
                "{marker} exists (finish the merge or rebase first)"
            ));
        }
    }
    match git(root, &["status", "--porcelain", "--", OUT]) {
        Ok(status) if status.is_empty() => None,
        Ok(_) => Some(format!("{OUT} has uncommitted edits")),
        Err(e) => Some(format!("git status: {e}")),
    }
}

pub(crate) fn commit_message(date: &str, packages: usize, reason: Reason) -> String {
    format!(
        "licenses: regenerate licenses.html ({date}, {packages} packages, {})",
        reason.as_str()
    )
}

/// Commit just the page and the texts dir: `--only` leaves whatever
/// else the operator has staged out of this commit.
fn commit(root: &Path, message: &str) -> Result<String, String> {
    git(root, &["add", "--", OUT, TEXTS]).map_err(|e| format!("git add: {e}"))?;
    git(
        root,
        &["commit", "--only", "-q", "-m", message, "--", OUT, TEXTS],
    )
    .map_err(|e| format!("git commit: {e}"))?;
    git(root, &["rev-parse", "--short", "HEAD"])
}

// ---- entry points -------------------------------------------------------------

#[derive(Default, Clone, Copy)]
pub(crate) struct Options {
    pub(crate) force: bool,
    pub(crate) check: bool,
    pub(crate) no_commit: bool,
}

#[derive(Debug)]
pub(crate) enum Outcome {
    Fresh {
        generated: String,
    },
    Stale(Reason),
    Written {
        reason: Reason,
        packages: usize,
        commit: Result<String, String>,
    },
}

/// Regenerate `root`'s page when stale (or forced) and commit it unless
/// told not to. `--check` only reports.
pub(crate) fn refresh(root: &Path, opts: Options) -> Result<Outcome, String> {
    let out = root.join(OUT);
    let existing = std::fs::read_to_string(&out).ok();
    let inputs = inputs_hash(root)?;
    let now = crate::unix_now();
    let reason = if opts.force {
        Some(Reason::Forced)
    } else {
        stale(existing.as_deref(), &inputs, now)
    };
    let Some(reason) = reason else {
        let generated = existing
            .as_deref()
            .and_then(|h| meta(h, "bom:generated"))
            .unwrap_or_default();
        return Ok(Outcome::Fresh { generated });
    };
    if opts.check {
        return Ok(Outcome::Stale(reason));
    }
    let blocker = if opts.no_commit {
        Some("--no-commit".to_string())
    } else {
        commit_blocker(root)
    };

    let mut packages = cargo::collect(root)?;
    packages.extend(npm::collect(root)?);
    packages.extend(extra::collect(root)?);
    let mut lib = texts::Library::load(&root.join(TEXTS))?;
    let mut licences = BTreeSet::new();
    let mut exceptions = BTreeSet::new();
    for pkg in &packages {
        let (l, e) = texts::ids(&pkg.license);
        licences.extend(l);
        exceptions.extend(e);
    }
    lib.ensure(&licences, &exceptions)?;
    let count = packages.len();
    let bom = assemble(packages, &lib);
    // Hash after `ensure`: newly fetched texts are inputs too.
    let inputs = inputs_hash(root)?;
    let generated = crate::setup::brand::rfc3339(now);
    let html = render::page(&bom, &lib, &generated, &inputs);
    std::fs::write(&out, html).map_err(|e| format!("write {}: {e}", out.display()))?;
    println!(
        "licenses: wrote {OUT} ({count} packages, {} canonical texts, {} variants, {} notices; {})",
        bom.canonical.len(),
        bom.variants.len(),
        bom.notices.len(),
        reason.as_str()
    );
    if !lib.fetched.is_empty() {
        println!("licenses: new texts in {TEXTS}: {}", lib.fetched.join(", "));
    }
    let commit = match blocker {
        Some(why) => Err(why),
        None => commit(root, &commit_message(&generated[..10], count, reason)),
    };
    Ok(Outcome::Written {
        reason,
        packages: count,
        commit,
    })
}

pub(crate) fn run(args: &[String]) {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "Usage: cargo xtask licenses [--force] [--check] [--no-commit] [--root <dir>]\n\n\
             Regenerate {OUT} when it is 7 days old or its inputs changed, and commit it.\n  \
             --force      regenerate now\n  \
             --check      exit 1 if stale; write nothing\n  \
             --no-commit  leave the regenerated page uncommitted\n  \
             --root <dir> repo checkout to work on (default: .)"
        );
        return;
    }
    let has = |name: &str| args.iter().any(|a| a == name);
    let opts = Options {
        force: has("--force"),
        check: has("--check"),
        no_commit: has("--no-commit"),
    };
    let root = PathBuf::from(flag(args, "root").unwrap_or_else(|| ".".to_string()));
    match refresh(&root, opts) {
        Ok(Outcome::Fresh { generated }) => {
            println!("licenses: fresh (generated {generated})");
        }
        Ok(Outcome::Stale(reason)) => {
            eprintln!("licenses: {OUT} is stale ({})", reason.as_str());
            std::process::exit(1);
        }
        Ok(Outcome::Written {
            reason,
            packages,
            commit,
        }) => match commit {
            Ok(sha) => println!(
                "licenses: committed {sha} ({packages} packages, {})",
                reason.as_str()
            ),
            Err(why) => println!("licenses: not committed: {why}"),
        },
        Err(e) => {
            eprintln!("licenses: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_round_trips() {
        for unix in [0, 951_782_400, 1_793_750_400, 1_793_836_799] {
            let s = crate::setup::brand::rfc3339(unix);
            assert_eq!(parse_rfc3339(&s), Some(unix), "{s}");
        }
        assert_eq!(parse_rfc3339("2026-11-04 12:00:00Z"), None);
        assert_eq!(parse_rfc3339("2026-13-04T12:00:00Z"), None);
    }

    #[test]
    fn staleness_is_age_or_inputs() {
        let page = |generated: &str, inputs: &str| {
            format!(
                "<head><meta name=\"bom:generated\" content=\"{generated}\">\n\
                 <meta name=\"bom:inputs\" content=\"{inputs}\"></head>"
            )
        };
        let now = parse_rfc3339("2026-11-11T12:00:00Z").unwrap();
        let fresh = page("2026-11-05T12:00:01Z", "sha256:a");
        assert_eq!(stale(Some(&fresh), "sha256:a", now), None);
        assert_eq!(
            stale(Some(&fresh), "sha256:b", now),
            Some(Reason::InputsChanged)
        );
        let old = page("2026-11-04T12:00:00Z", "sha256:a");
        assert_eq!(stale(Some(&old), "sha256:a", now), Some(Reason::Age));
        assert_eq!(
            stale(Some("<html>"), "sha256:a", now),
            Some(Reason::InputsChanged)
        );
        assert_eq!(stale(None, "sha256:a", now), Some(Reason::Missing));
    }

    #[test]
    fn commit_message_names_date_count_and_reason() {
        assert_eq!(
            commit_message("2026-11-04", 612, Reason::Age),
            "licenses: regenerate licenses.html (2026-11-04, 612 packages, 7-day refresh)"
        );
    }

    fn pkg(name: &str, license: &str, files: &[(FileKind, &str)]) -> Package {
        Package {
            ecosystem: "crates.io".into(),
            name: name.into(),
            version: "1.0.0".into(),
            url: format!("https://github.com/x/{name}"),
            license: license.into(),
            scopes: BTreeSet::from([Scope::App]),
            copyrights: Vec::new(),
            authors: Vec::new(),
            files: files
                .iter()
                .map(|(kind, text)| LicenseFile {
                    kind: *kind,
                    text: text.to_string(),
                })
                .collect(),
            note: None,
        }
    }

    #[test]
    fn assemble_shares_canonical_texts_and_keeps_copyrights_per_row() {
        use texts::tests::{ISC, MIT, library};
        let apache = "Apache License Version 2.0 stand-in: terms and conditions for use, \
                      reproduction and distribution, with definitions of licensor, legal entity, \
                      source form, object form, work, derivative works and contribution.";
        let lib = library(&[("MIT", MIT), ("ISC", ISC), ("Apache-2.0", apache)]);
        let alice = MIT.replace(
            "Copyright (c) <year> <copyright holders>",
            "Copyright 2019 Alice",
        );
        let bob = MIT.replace(
            "Copyright (c) <year> <copyright holders>",
            "Copyright (c) Bob",
        );
        let odd = format!(
            "{}\nAdditional term: you must also send a postcard to the authors \
             every year you ship this software in a commercial product, and keep a \
             copy of every postcard you send for at least seven years after shipping.",
            ISC.replace("<year> <copyright holders>", "2020 Carol")
        );
        let bom = assemble(
            vec![
                pkg("b", "MIT", &[(FileKind::License, &bob)]),
                pkg("a", "MIT OR Apache-2.0", &[(FileKind::License, &alice)]),
                pkg(
                    "c",
                    "ISC",
                    &[
                        (FileKind::License, &odd),
                        (FileKind::Notice, "Carol's notice"),
                    ],
                ),
                pkg("d", "", &[(FileKind::License, ISC)]),
            ],
            &lib,
        );
        let names: Vec<&str> = bom.rows.iter().map(|r| r.pkg.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c", "d"]);
        assert_eq!(bom.canonical["MIT"], vec![0, 1]);
        assert_eq!(bom.rows[0].pkg.copyrights, vec!["Copyright 2019 Alice"]);
        assert_eq!(bom.rows[1].pkg.copyrights, vec!["Copyright (c) Bob"]);
        // c's text differs beyond its copyright: a variant, linked from ISC.
        assert_eq!(bom.variants.len(), 1);
        let (anchor, variant) = bom.variants.iter().next().unwrap();
        assert_eq!(bom.rows[2].links["ISC"], *anchor);
        assert!(!variant.text.contains("Carol"));
        assert_eq!(bom.rows[2].notices.len(), 1);
        // d declares nothing; its file reads as ISC.
        assert!(bom.rows[3].detected);
        assert_eq!(bom.rows[3].expr, "ISC");
        assert_eq!(bom.canonical["ISC"], vec![3]);
    }

    #[test]
    fn scan_dir_classifies_files() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "LICENSE-MIT",
            "NOTICE",
            "COPYRIGHT",
            "license.js",
            "README.md",
        ] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        std::fs::create_dir(dir.path().join("LICENSES")).unwrap();
        std::fs::write(dir.path().join("LICENSES/Apache-2.0.txt"), "apache").unwrap();
        let kinds: Vec<(FileKind, String)> = scan_dir(dir.path())
            .into_iter()
            .map(|f| (f.kind, f.text))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (FileKind::Copyright, "COPYRIGHT".to_string()),
                (FileKind::License, "LICENSE-MIT".to_string()),
                (FileKind::License, "apache".to_string()),
                (FileKind::Notice, "NOTICE".to_string()),
            ]
        );
    }

    /// A scratch repo with one commit, `licenses.html` tracked.
    fn scratch_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let run = |args: &[&str]| git(root, args).unwrap();
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "bom@example.test"]);
        run(&["config", "user.name", "BOM test"]);
        run(&["config", "core.hooksPath", "/dev/null"]);
        std::fs::create_dir_all(root.join("crates/web/static")).unwrap();
        std::fs::create_dir_all(root.join(TEXTS)).unwrap();
        std::fs::write(root.join(OUT), "old").unwrap();
        std::fs::write(root.join(TEXTS).join("MIT.txt"), "mit").unwrap();
        std::fs::write(root.join("other.txt"), "base").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-q", "-m", "base"]);
        dir
    }

    #[test]
    fn commit_takes_only_the_page_and_texts() {
        let dir = scratch_repo();
        let root = dir.path();
        assert_eq!(commit_blocker(root), None);
        std::fs::write(root.join("other.txt"), "staged edit").unwrap();
        git(root, &["add", "other.txt"]).unwrap();
        std::fs::write(root.join(OUT), "new page").unwrap();
        std::fs::write(root.join(TEXTS).join("ISC.txt"), "isc").unwrap();
        commit(root, "licenses: test").unwrap();
        let files = git(root, &["show", "--name-only", "--format=", "HEAD"]).unwrap();
        assert_eq!(files, format!("{OUT}\n{TEXTS}/ISC.txt"));
        let staged = git(root, &["diff", "--cached", "--name-only"]).unwrap();
        assert_eq!(
            staged, "other.txt",
            "the operator's staged work stays staged"
        );
    }

    #[test]
    fn commit_is_blocked_by_edits_detached_head_and_missing_git() {
        let dir = scratch_repo();
        let root = dir.path();
        std::fs::write(root.join(OUT), "hand edit").unwrap();
        assert!(commit_blocker(root).unwrap().contains("uncommitted edits"));
        git(root, &["checkout", "-q", "--", OUT]).unwrap();
        git(root, &["checkout", "-q", "--detach"]).unwrap();
        assert_eq!(commit_blocker(root).as_deref(), Some("HEAD is detached"));
        let bare = tempfile::tempdir().unwrap();
        assert_eq!(
            commit_blocker(bare.path()).as_deref(),
            Some("not a git checkout")
        );
    }
}
