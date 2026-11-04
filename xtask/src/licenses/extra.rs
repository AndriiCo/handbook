//! Rows no lockfile pins: `docs/licenses/extra.toml`, the toolchain
//! pins it points at, and the Dockerfiles' `FROM` lines.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use super::{FileKind, LicenseFile, Package, Scope};

pub(crate) const EXTRA: &str = "docs/licenses/extra.toml";

const ECOSYSTEMS: &[&str] = &["toolchain", "apt", "pip", "binary", "model", "font"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtraFile {
    #[serde(default)]
    pub(crate) dockerfiles: BTreeMap<String, String>,
    #[serde(default)]
    image: Vec<Image>,
    #[serde(default)]
    package: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Image {
    name: String,
    url: String,
    license: String,
    copyright: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
    ecosystem: String,
    version: Option<String>,
    version_from: Option<String>,
    url: String,
    license: String,
    copyright: Option<String>,
    scope: Vec<String>,
    note: Option<String>,
    /// A licence file in this repo that ships with the thing (matched
    /// and scanned for copyrights like a package's own files).
    license_file: Option<String>,
}

pub(crate) fn load(root: &Path) -> Result<ExtraFile, String> {
    let path = root.join(EXTRA);
    let raw =
        std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    toml::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))
}

pub(crate) fn collect(root: &Path) -> Result<Vec<Package>, String> {
    let file = load(root)?;
    let mut out = Vec::new();
    for entry in &file.package {
        if !ECOSYSTEMS.contains(&entry.ecosystem.as_str()) {
            return Err(format!(
                "{EXTRA}: {}: ecosystem {:?} is not one of {}",
                entry.name,
                entry.ecosystem,
                ECOSYSTEMS.join(", ")
            ));
        }
        let version = match (&entry.version, &entry.version_from) {
            (Some(v), None) => v.clone(),
            (None, Some(from)) => version_from(root, from)?,
            _ => {
                return Err(format!(
                    "{EXTRA}: {}: set exactly one of version, version_from",
                    entry.name
                ));
            }
        };
        let files = match &entry.license_file {
            Some(rel) => vec![LicenseFile {
                kind: FileKind::License,
                text: std::fs::read_to_string(root.join(rel))
                    .map_err(|e| format!("{EXTRA}: {}: read {rel}: {e}", entry.name))?,
            }],
            None => Vec::new(),
        };
        out.push(Package {
            ecosystem: entry.ecosystem.clone(),
            name: entry.name.clone(),
            version,
            url: entry.url.clone(),
            license: entry.license.clone(),
            scopes: parse_scopes(&entry.name, &entry.scope)?,
            copyrights: entry.copyright.iter().cloned().collect(),
            authors: Vec::new(),
            files,
            note: entry.note.clone(),
        });
    }
    out.extend(images(root, &file)?);
    Ok(out)
}

fn parse_scopes(name: &str, raw: &[String]) -> Result<BTreeSet<Scope>, String> {
    if raw.is_empty() {
        return Err(format!("{EXTRA}: {name}: scope is empty"));
    }
    raw.iter()
        .map(|s| Scope::parse(s).ok_or_else(|| format!("{EXTRA}: {name}: unknown scope {s:?}")))
        .collect()
}

/// A version read from a pin file in the repo.
fn version_from(root: &Path, file: &str) -> Result<String, String> {
    let raw = std::fs::read_to_string(root.join(file)).map_err(|e| format!("read {file}: {e}"))?;
    match file {
        "rust-toolchain.toml" => {
            let doc: toml::Value =
                toml::from_str(&raw).map_err(|e| format!("parse {file}: {e}"))?;
            doc["toolchain"]["channel"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{file}: no toolchain.channel"))
        }
        ".nvmrc" => Ok(raw.trim().trim_start_matches('v').to_string()),
        other => Err(format!(
            "{EXTRA}: version_from {other:?} is not a known pin file"
        )),
    }
}

/// One row per base image `name:tag` across the listed Dockerfiles.
fn images(root: &Path, file: &ExtraFile) -> Result<Vec<Package>, String> {
    let mut rows: BTreeMap<String, Package> = BTreeMap::new();
    for (dockerfile, scope) in &file.dockerfiles {
        let scope = Scope::parse(scope)
            .ok_or_else(|| format!("{EXTRA}: {dockerfile}: unknown scope {scope:?}"))?;
        let raw = std::fs::read_to_string(root.join(dockerfile))
            .map_err(|e| format!("read {dockerfile}: {e}"))?;
        let mut stages: BTreeSet<String> = BTreeSet::new();
        for reference in from_lines(&raw) {
            let (image, stage) = reference;
            if let Some(stage) = stage {
                stages.insert(stage);
            }
            if stages.contains(&image) && !image.contains(':') {
                continue; // FROM <earlier stage>
            }
            let (name, tag) = image.split_once(':').unwrap_or((image.as_str(), "latest"));
            let tag = tag.split('@').next().unwrap_or(tag);
            let Some(known) = file.image.iter().find(|i| i.name == name) else {
                return Err(format!(
                    "{dockerfile}: base image {name:?} has no [[image]] entry in {EXTRA}"
                ));
            };
            let row = rows.entry(image.clone()).or_insert_with(|| Package {
                ecosystem: "docker".into(),
                name: name.to_string(),
                version: tag.to_string(),
                url: known.url.clone(),
                license: known.license.clone(),
                scopes: BTreeSet::new(),
                copyrights: known.copyright.iter().cloned().collect(),
                authors: Vec::new(),
                files: Vec::new(),
                note: None,
            });
            row.scopes.insert(scope);
            let note = format!("Base image in {dockerfile}");
            row.note = Some(match row.note.take() {
                Some(prev) if !prev.contains(dockerfile.as_str()) => {
                    format!("{prev}; {dockerfile}")
                }
                Some(prev) => prev,
                None => note,
            });
        }
    }
    Ok(rows.into_values().collect())
}

/// `(image, stage alias)` of every `FROM` line (flags skipped).
fn from_lines(dockerfile: &str) -> Vec<(String, Option<String>)> {
    dockerfile
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            if !words.next()?.eq_ignore_ascii_case("FROM") {
                return None;
            }
            let rest: Vec<&str> = words.filter(|w| !w.starts_with("--")).collect();
            let image = rest.first()?.to_string();
            let stage = match rest.as_slice() {
                [_, as_kw, alias, ..] if as_kw.eq_ignore_ascii_case("AS") => {
                    Some(alias.to_string())
                }
                _ => None,
            };
            Some((image, stage))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_lines_read_images_and_stage_aliases() {
        let df = "# syntax=docker/dockerfile:1\nFROM node:24-slim AS web\nRUN x\n\
                  FROM --platform=linux/amd64 rust:1.99.0-bookworm AS rust\nFROM web\n\
                  from debian:bookworm-slim\n";
        assert_eq!(
            from_lines(df),
            vec![
                ("node:24-slim".to_string(), Some("web".to_string())),
                ("rust:1.99.0-bookworm".to_string(), Some("rust".to_string())),
                ("web".to_string(), None),
                ("debian:bookworm-slim".to_string(), None),
            ]
        );
    }

    #[test]
    fn the_committed_extra_file_parses_and_resolves() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let rows = collect(root).unwrap();
        let rust = rows
            .iter()
            .find(|p| p.name.starts_with("Rust toolchain"))
            .unwrap();
        assert!(rust.version.starts_with("1."), "{}", rust.version);
        assert!(
            rows.iter()
                .any(|p| p.ecosystem == "docker" && p.name == "debian")
        );
    }
}
