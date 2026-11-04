//! Jurisdiction packs (F13): parse, validate, and load `packs/*.json`.
//!
//! A pack is data: authorities, domain verdicts, thin-watch seeds,
//! chain steps, and cited-but-unverified observations. Loading a pack
//! writes `authority`, `domain`, `source`, and `task` rows only — it
//! NEVER writes `fact` or `capture` rows, so no pack value can reach
//! customers until a staff-verified capture plus F4 approval exists.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use contracts::topics::Topic;
use rusqlite::{Connection, OptionalExtension};

/// Pack schema version this loader understands.
const PACK_VERSION: u32 = 1;

/// The only accepted status for chain steps and observations.
/// Anything else means "presented as usable" and fails validation.
const NEEDS_REVIEW: &str = "needs_review";

/// Canonical PO-Box fixture values. They must never appear in any
/// pack (specs/README.md resolution 9); `PO Box 40228` in usa-wa is
/// a cited observation from the superseded form, not a fixture.
const BANNED_FIXTURES: [&str; 2] = ["40220", "40234"];

/// Seed-file parse or validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackError {
    /// Pack id (or `?` when the file never parsed that far).
    pub pack: String,
    /// One line per violation.
    pub violations: Vec<String>,
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "pack {}: {}", self.pack, self.violations.join("; "))
    }
}

impl std::error::Error for PackError {}

/// One pack file (`packs/<pack>.json`), schema version 1.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Pack {
    /// Unique id; must equal the filename stem.
    pub pack: String,
    /// Must equal [`PACK_VERSION`].
    pub version: u32,
    /// Human title (docs only).
    #[serde(default)]
    pub title: String,
    /// `authority` rows to seed.
    #[serde(default)]
    pub authorities: Vec<PackAuthority>,
    /// `domain` rows to seed (anti-spoofing verdicts).
    #[serde(default)]
    pub domains: Vec<PackDomain>,
    /// `source` rows to seed (thin-watch ingestion seeds).
    #[serde(default)]
    pub watch: Vec<PackWatch>,
    /// Chain skeleton; each becomes an open `validate_fact` task.
    #[serde(default)]
    pub chain_steps: Vec<PackStep>,
    /// Cited-but-unverified values; each becomes an open task and
    /// NEVER a fact row.
    #[serde(default)]
    pub observations: Vec<PackObservation>,
}

/// One `authority` row seed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PackAuthority {
    /// `country | state | province | office` (migration 005).
    pub kind: String,
    /// Free text; no country-code enums anywhere (F13).
    pub country: String,
    /// Free text, may be empty.
    #[serde(default)]
    pub region: String,
    /// Display name.
    pub name: String,
    /// Unique slug (join key for the rest of the pack).
    pub slug: String,
}

/// One `domain` row seed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PackDomain {
    /// Hostname verdict subject.
    pub name: String,
    /// Owning authority slug.
    pub authority: String,
    /// `official | known_thirdparty | unverified | fake`.
    pub status: String,
    /// Why this verdict (evidence pointer).
    #[serde(default)]
    pub notes: String,
}

/// One thin-watch seed → one `source` row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PackWatch {
    /// Owning authority slug.
    pub authority: String,
    /// `website_official | website_thirdparty | …` (migration 005).
    pub kind: String,
    /// Absolute `http(s)` URL to poll.
    pub uri: String,
    /// `hourly | daily | weekly`; stored as a `cadence:` prefix in
    /// `source.notes` (the table has no cadence column).
    pub cadence: String,
    /// Operator note (fetch hazards, validation refs).
    #[serde(default)]
    pub notes: String,
}

/// One chain step; stays `needs_review` until a staff-verified
/// capture plus F4 approval exists for its values.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PackStep {
    /// Stable id; part of the task `dedup_key`.
    pub id: String,
    /// Owning authority slug.
    pub authority: String,
    /// Visit order; unique within the pack.
    pub order: u32,
    /// Short step name.
    pub title: String,
    /// What is (un)verified about this step.
    #[serde(default)]
    pub detail: String,
    /// Must be exactly `needs_review`.
    pub status: String,
    /// Backlog ref (`V-1…V-19`, `P3-a/b`, `F2`).
    pub validation: String,
}

/// One cited-but-unverified value. Loaded as a task, never a fact.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PackObservation {
    /// Owning authority slug.
    pub authority: String,
    /// One of the 25 [`Topic`] wire strings.
    pub topic: String,
    /// The observed claim (must read as observed, not asserted).
    pub statement: String,
    /// URL the claim was read at.
    pub source_uri: String,
    /// Must be exactly `needs_review`.
    pub status: String,
    /// Backlog ref (`V-1…V-19`, `P3-a/b`, `F2`).
    pub validation: String,
}

/// Pack v2 per-authority attributes (all optional and additive).
///
/// Read from the same `authorities[]` objects as [`PackAuthority`]
/// (each view ignores the keys it does not name), so v1 files stay
/// valid and code that builds a v1 [`Pack`] keeps compiling.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthorityExt {
    /// Join key back to the [`PackAuthority`] at the same index (the
    /// writer takes it from there, so it is never written twice).
    #[serde(skip_serializing)]
    pub slug: String,
    /// `true` hides the row from every public read (`hidden_at`);
    /// `false` unhides it; absent leaves it as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// This entry IS another pack's authority (a world row for a
    /// curated US state): the load attaches this pack's links and
    /// tasks to that slug's row and writes no row of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merges_into: Option<String>,
    /// ISO 3166-1 alpha-2; overrides the pack-level `iso2`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub iso2: String,
    /// ISO 3166-2 (`US-WA`, `DE-BY`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subdivision: String,
    /// One of [`LEVELS`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub level: String,
    /// English display name when `name` is in another language.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name_en: String,
    /// Other search terms ("Bayern", "Deutschland").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// Readable addresses, primary first (`usa/new_york/chemung`),
    /// written to `authority_path`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Document routes this office handles (`authority_scope` rows plus
    /// one review task; never facts).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<ScopeEntry>,
    /// The verbatim extracted name a curated `name` replaced (world
    /// index provenance: a re-run matches the row by it).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_name: String,
}

/// `authority.level` vocabulary (migration 013).
pub const LEVELS: [&str; 5] = ["federal", "state", "regional", "local", "consular"];

/// The closed list of document classes a route can name (P4-1).
pub const DOC_CLASSES: [&str; 9] = [
    "vital_record",
    "state_court",
    "federal_court",
    "notarized_private",
    "company",
    "state_agency",
    "federal_agency",
    "education",
    "fbi_check",
];

/// One document route a pack proposes: this office apostilles
/// `doc_class` documents issued in `issued_in` (ISO 3166-1 or 3166-2,
/// e.g. `US`, `US-WA`), after `prereq`, as read at `source_uri`. It
/// stays unverified on place pages until staff accept a `jurisdiction`
/// fact `doc_rule: <doc_class> …` for the office.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScopeEntry {
    pub doc_class: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub issued_in: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prereq: String,
    pub source_uri: String,
}

/// `US` or `US-WA` (or empty).
fn is_place_code(code: &str) -> bool {
    match code.split_once('-') {
        None => code.is_empty() || is_iso2(code),
        Some((country, sub)) => {
            is_iso2(country)
                && (1..=3).contains(&sub.len())
                && sub
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        }
    }
}

/// Pack v2 view of one file: pack-level defaults plus one
/// [`AuthorityExt`] per `authorities[]` entry, in the same order.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct PackExt {
    /// Default ISO 3166-1 alpha-2 for every authority in the pack.
    #[serde(default)]
    pub iso2: String,
    #[serde(default)]
    pub authorities: Vec<AuthorityExt>,
}

/// One pack file: the v1 shape plus its v2 view.
#[derive(Debug, Clone)]
pub struct PackFile {
    pub pack: Pack,
    pub ext: PackExt,
}

impl PackFile {
    /// Parse both views of one file.
    ///
    /// # Errors
    ///
    /// The serde error when either view does not parse.
    pub fn parse(bytes: &[u8]) -> serde_json::Result<Self> {
        Ok(Self {
            pack: serde_json::from_slice(bytes)?,
            ext: serde_json::from_slice(bytes)?,
        })
    }

    /// A v1 pack with no v2 attributes.
    #[cfg(test)]
    #[must_use]
    pub fn v1(pack: Pack) -> Self {
        Self {
            pack,
            ext: PackExt::default(),
        }
    }

    /// The v2 attributes of `authorities[i]` (defaults when absent).
    #[must_use]
    pub fn ext_of(&self, i: usize) -> AuthorityExt {
        self.ext.authorities.get(i).cloned().unwrap_or_default()
    }

    /// The effective ISO code of `authorities[i]`.
    #[must_use]
    pub fn iso2_of(&self, i: usize) -> String {
        let own = self.ext_of(i).iso2;
        if own.is_empty() {
            self.ext.iso2.clone()
        } else {
            own
        }
    }

    /// Pretty JSON in the committed layout: v1 keys in their order,
    /// each authority's v2 keys after its `slug`, trailing newline.
    #[must_use]
    pub fn render(&self) -> String {
        #[derive(serde::Serialize)]
        struct AuthorityOut<'a> {
            #[serde(flatten)]
            base: &'a PackAuthority,
            #[serde(flatten)]
            ext: AuthorityExt,
        }
        #[derive(serde::Serialize)]
        struct PackOut<'a> {
            pack: &'a str,
            version: u32,
            #[serde(skip_serializing_if = "str::is_empty")]
            iso2: &'a str,
            title: &'a str,
            authorities: Vec<AuthorityOut<'a>>,
            domains: &'a [PackDomain],
            watch: &'a [PackWatch],
            chain_steps: &'a [PackStep],
            observations: &'a [PackObservation],
        }
        let pack = &self.pack;
        let out = PackOut {
            pack: &pack.pack,
            version: pack.version,
            iso2: &self.ext.iso2,
            title: &pack.title,
            authorities: pack
                .authorities
                .iter()
                .enumerate()
                .map(|(i, base)| AuthorityOut {
                    base,
                    ext: self.ext_of(i),
                })
                .collect(),
            domains: &pack.domains,
            watch: &pack.watch,
            chain_steps: &pack.chain_steps,
            observations: &pack.observations,
        };
        let mut text = serde_json::to_string_pretty(&out).unwrap_or_default();
        text.push('\n');
        text
    }
}

/// Why a display name reads as a fragment of a page rather than the
/// name of an office (the store's [`authority_name_lint`], shared with
/// the staff screen), or `None` for a plausible name. Hidden rows are
/// exempt: they exist so a load can hide the production row.
///
/// [`authority_name_lint`]: handbook_store::db::authority_name_lint
#[must_use]
pub fn name_lint(name: &str) -> Option<&'static str> {
    handbook_store::db::authority_name_lint(name)
}

/// `usa/New York` → `usa/new_york`-style check: 1-3 segments of
/// `[a-z0-9_-]` (the web resolver's address shape).
fn is_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    (1..=3).contains(&segments.len())
        && segments.iter().all(|s| {
            !s.is_empty()
                && s.len() <= 48
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        })
}

fn is_iso2(code: &str) -> bool {
    code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase())
}

/// Sub-directories of a pack root that hold further pack sets, in
/// load order after the curated root (world rows that are a curated
/// office say so with `merges_into`).
pub const TREE_SUBDIRS: [&str; 2] = ["world", "world-nonparty"];

/// Read and validate every `*.json` pack in `dir`, plus the slug rules
/// across those files (one owner per slug).
///
/// # Errors
///
/// Returns one [`PackError`] per bad file (bad JSON or a rule
/// violation). An empty vec means every pack is clean.
#[must_use]
pub fn validate_dir(dir: &Path) -> Vec<PackError> {
    validate_dirs(&[dir.to_path_buf()], false)
}

/// Validate a whole pack tree: `root` (the curated set) plus each of
/// [`TREE_SUBDIRS`] that exists, with the slug rules across all of
/// them: a slug is declared by exactly one pack, and every
/// `merges_into` names a slug some pack in the tree declares. This is
/// what `packs-validate` and the deploy gate run.
#[must_use]
pub fn validate_tree(root: &Path) -> Vec<PackError> {
    validate_dirs(&tree_dirs(root), true)
}

/// Validate a staged set (`<store>/packs-inbox/<id>`) as if it were
/// part of the tree at `root`: its slugs must not be owned by a
/// release pack, and its `merges_into` targets must exist there.
#[must_use]
pub fn validate_against(dir: &Path, root: &Path) -> Vec<PackError> {
    let mut dirs = tree_dirs(root);
    dirs.push(dir.to_path_buf());
    validate_dirs(&dirs, true)
}

fn tree_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    for sub in TREE_SUBDIRS {
        let dir = root.join(sub);
        if dir.is_dir() {
            dirs.push(dir);
        }
    }
    dirs
}

/// Number of `*.json` packs `validate_tree(root)` reads.
#[must_use]
pub fn tree_pack_count(root: &Path) -> usize {
    std::iter::once(root.to_path_buf())
        .chain(TREE_SUBDIRS.iter().map(|sub| root.join(sub)))
        .filter_map(|dir| pack_paths(&dir).ok())
        .map(|paths| paths.len())
        .sum()
}

/// Sorted `*.json` paths in `dir`.
fn pack_paths(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    Ok(paths)
}

fn validate_dirs(dirs: &[PathBuf], whole_tree: bool) -> Vec<PackError> {
    let mut errors = Vec::new();
    let mut parsed: Vec<PackFile> = Vec::new();
    for dir in dirs {
        let paths = match pack_paths(dir) {
            Ok(paths) => paths,
            Err(e) => {
                errors.push(PackError {
                    pack: dir.display().to_string(),
                    violations: vec![format!("cannot list packs dir: {e}")],
                });
                continue;
            }
        };
        if paths.is_empty() {
            errors.push(PackError {
                pack: dir.display().to_string(),
                violations: vec![String::from("no *.json packs found")],
            });
        }
        for path in paths {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("?")
                .to_string();
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    errors.push(PackError {
                        pack: stem,
                        violations: vec![format!("cannot read {}: {e}", path.display())],
                    });
                    continue;
                }
            };
            match PackFile::parse(&bytes) {
                Ok(file) => {
                    let violations = validate(&file.pack, &stem, &bytes);
                    if !violations.is_empty() {
                        errors.push(PackError {
                            pack: file.pack.pack.clone(),
                            violations,
                        });
                    }
                    parsed.push(file);
                }
                Err(e) => errors.push(PackError {
                    pack: stem,
                    violations: vec![format!("invalid JSON in {}: {e}", path.display())],
                }),
            }
        }
    }
    errors.extend(cross_pack_violations(&parsed, whole_tree));
    errors
}

/// Slug ownership across packs: each slug is declared once (a second
/// declaration would make load order decide its name, since a load
/// updates the row); a `merges_into` entry's own slug is local to its
/// pack; with `whole_tree`, every `merges_into` target is declared.
fn cross_pack_violations(files: &[PackFile], whole_tree: bool) -> Vec<PackError> {
    let mut owner: HashMap<&str, &str> = HashMap::new();
    let mut out: Vec<PackError> = Vec::new();
    let mut push = |pack: &str, violation: String| match out.iter_mut().find(|e| e.pack == pack) {
        Some(error) => error.violations.push(violation),
        None => out.push(PackError {
            pack: pack.to_string(),
            violations: vec![violation],
        }),
    };
    for file in files {
        for (i, authority) in file.pack.authorities.iter().enumerate() {
            if file.ext_of(i).merges_into.is_some() {
                continue;
            }
            if let Some(first) = owner.insert(&authority.slug, &file.pack.pack)
                && first != file.pack.pack
            {
                push(
                    &file.pack.pack,
                    format!(
                        "authority {:?} is also declared by pack {first:?}: a slug has one \
                         owner (use \"merges_into\" for an entry that is that office)",
                        authority.slug
                    ),
                );
            }
        }
    }
    for file in files {
        for (i, authority) in file.pack.authorities.iter().enumerate() {
            let Some(target) = file.ext_of(i).merges_into else {
                continue;
            };
            if let Some(other) = owner.get(authority.slug.as_str()) {
                push(
                    &file.pack.pack,
                    format!(
                        "authority {:?} merges into {target:?} but its own slug is declared by \
                         pack {other:?}",
                        authority.slug
                    ),
                );
            }
            match owner.get(target.as_str()) {
                Some(other) if *other == file.pack.pack => push(
                    &file.pack.pack,
                    format!(
                        "authority {:?} merges into {target:?} from its own pack",
                        authority.slug
                    ),
                ),
                None if whole_tree => push(
                    &file.pack.pack,
                    format!(
                        "authority {:?} merges into {target:?}, which no pack declares",
                        authority.slug
                    ),
                ),
                _ => {}
            }
        }
    }
    out
}

/// `name` shortened for a violation line.
fn clip(name: &str) -> String {
    let mut out: String = name.chars().take(60).collect();
    if out.len() < name.len() {
        out.push('…');
    }
    out
}

/// Validate one parsed pack against every pack rule.
///
/// `stem` is the filename stem (must equal `pack.pack`); `raw` is
/// the file bytes (scanned for banned fixture values, and read again
/// for the v2 attributes in [`PackExt`]).
pub(crate) fn validate(pack: &Pack, stem: &str, raw: &[u8]) -> Vec<String> {
    let mut violations = Vec::new();
    if pack.pack != stem {
        violations.push(format!(
            "pack id {:?} does not match filename stem {stem:?}",
            pack.pack
        ));
    }
    if pack.version != PACK_VERSION {
        violations.push(format!("version {} is not {PACK_VERSION}", pack.version));
    }
    if pack.authorities.is_empty() {
        violations.push(String::from("at least one authority is required"));
    }
    let raw_text = String::from_utf8_lossy(raw);
    for banned in BANNED_FIXTURES {
        if raw_text.contains(banned) {
            violations.push(format!(
                "banned fixture value {banned} must never appear in a pack"
            ));
        }
    }
    // The first JSON value only: trailing bytes are the fixture scan's
    // business above, not a v2 parse error.
    let mut de = serde_json::Deserializer::from_slice(raw);
    let ext = match <PackExt as serde::Deserialize>::deserialize(&mut de) {
        Ok(ext) => ext,
        Err(e) => {
            violations.push(format!("pack v2 fields: {e}"));
            PackExt::default()
        }
    };
    if !ext.iso2.is_empty() && !is_iso2(&ext.iso2) {
        violations.push(format!(
            "iso2 {:?} is not an ISO 3166-1 alpha-2 code",
            ext.iso2
        ));
    }

    let mut slugs = HashSet::new();
    let mut paths: HashSet<String> = HashSet::new();
    for (i, authority) in pack.authorities.iter().enumerate() {
        let slug = authority.slug.as_str();
        if !matches!(
            authority.kind.as_str(),
            "country" | "state" | "province" | "office"
        ) {
            violations.push(format!("authority {slug:?}: bad kind {:?}", authority.kind));
        }
        if slug.trim().is_empty() || authority.name.trim().is_empty() {
            violations.push(String::from("authority slug and name must be non-empty"));
        }
        if !slugs.insert(slug) {
            violations.push(format!("duplicate authority slug {slug:?}"));
        }
        let ax = ext.authorities.get(i).cloned().unwrap_or_default();
        if ax.hidden != Some(true)
            && ax.merges_into.is_none()
            && let Some(why) = name_lint(&authority.name)
        {
            violations.push(format!(
                "authority {slug:?}: name {:?} {why}; name the office from the cited page \
                 or mark the row \"hidden\": true",
                clip(&authority.name)
            ));
        }
        if let Some(target) = &ax.merges_into {
            if target.trim().is_empty() || target == slug {
                violations.push(format!(
                    "authority {slug:?}: merges_into must name another pack's slug"
                ));
            }
            let own_row = ax.hidden.is_some()
                || !ax.paths.is_empty()
                || !ax.iso2.is_empty()
                || !ax.subdivision.is_empty()
                || !ax.level.is_empty()
                || !ax.name_en.is_empty()
                || !ax.aliases.is_empty();
            if own_row {
                violations.push(format!(
                    "authority {slug:?}: a merging entry has no row of its own \
                     (only routes; set row fields in the target's pack)"
                ));
            }
        }
        let iso = if ax.iso2.is_empty() {
            &ext.iso2
        } else {
            &ax.iso2
        };
        if !ax.iso2.is_empty() && !is_iso2(&ax.iso2) {
            violations.push(format!(
                "authority {slug:?}: iso2 {:?} is not an ISO 3166-1 alpha-2 code",
                ax.iso2
            ));
        }
        if !ax.subdivision.is_empty() {
            let ok = ax
                .subdivision
                .split_once('-')
                .is_some_and(|(country, sub)| {
                    country == iso
                        && (1..=3).contains(&sub.len())
                        && sub
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
                });
            if !ok {
                violations.push(format!(
                    "authority {slug:?}: subdivision {:?} is not ISO 3166-2 under {iso:?}",
                    ax.subdivision
                ));
            }
        }
        if !ax.level.is_empty() && !LEVELS.contains(&ax.level.as_str()) {
            violations.push(format!(
                "authority {slug:?}: bad level {:?} (one of {LEVELS:?})",
                ax.level
            ));
        }
        for alias in &ax.aliases {
            if alias.trim().is_empty() || alias.contains('\n') {
                violations.push(format!("authority {slug:?}: empty or multi-line alias"));
            }
        }
        let mut routes = HashSet::new();
        for route in &ax.scope {
            if !DOC_CLASSES.contains(&route.doc_class.as_str()) {
                violations.push(format!(
                    "authority {slug:?}: unknown doc_class {:?} (one of {DOC_CLASSES:?})",
                    route.doc_class
                ));
            }
            if !is_place_code(&route.issued_in) {
                violations.push(format!(
                    "authority {slug:?}: issued_in {:?} is not ISO 3166-1/-2",
                    route.issued_in
                ));
            }
            if !is_http_url(&route.source_uri) {
                violations.push(format!(
                    "authority {slug:?}: route source_uri {:?} is not absolute http(s)",
                    route.source_uri
                ));
            }
            if !routes.insert((route.doc_class.as_str(), route.issued_in.as_str())) {
                violations.push(format!(
                    "authority {slug:?}: duplicate route {} / {:?}",
                    route.doc_class, route.issued_in
                ));
            }
        }
        for path in &ax.paths {
            if !is_path(path) {
                violations.push(format!(
                    "authority {slug:?}: path {path:?} is not 1-3 segments of [a-z0-9_-]"
                ));
            }
            if !paths.insert(path.clone()) {
                violations.push(format!("duplicate path {path:?}"));
            }
        }
    }

    let mut hosts = HashSet::new();
    for domain in &pack.domains {
        if !slugs.contains(domain.authority.as_str()) {
            violations.push(format!(
                "domain {:?}: unknown authority {:?}",
                domain.name, domain.authority
            ));
        }
        if !matches!(
            domain.status.as_str(),
            "official" | "known_thirdparty" | "unverified" | "fake"
        ) {
            violations.push(format!(
                "domain {:?}: bad status {:?}",
                domain.name, domain.status
            ));
        }
        if domain.name.trim().is_empty() {
            violations.push(String::from("domain name must be non-empty"));
        }
        hosts.insert(domain.name.to_lowercase());
    }

    for seed in &pack.watch {
        if !slugs.contains(seed.authority.as_str()) {
            violations.push(format!(
                "watch {:?}: unknown authority {:?}",
                seed.uri, seed.authority
            ));
        }
        if !matches!(
            seed.kind.as_str(),
            "website_official"
                | "website_thirdparty"
                | "email_official"
                | "email_customer"
                | "phone"
                | "visit"
                | "comment"
                | "document"
        ) {
            violations.push(format!("watch {:?}: bad kind {:?}", seed.uri, seed.kind));
        }
        if !matches!(seed.cadence.as_str(), "hourly" | "daily" | "weekly") {
            violations.push(format!(
                "watch {:?}: bad cadence {:?}",
                seed.uri, seed.cadence
            ));
        }
        if !is_http_url(&seed.uri) {
            violations.push(format!("watch uri {:?} is not absolute http(s)", seed.uri));
        } else if let Some(host) = uri_host(&seed.uri)
            && !hosts.contains(&host)
        {
            violations.push(format!(
                "watch {:?}: host {host:?} has no verdict row in this pack's domains",
                seed.uri
            ));
        }
    }

    let mut orders = HashSet::new();
    let mut step_ids = HashSet::new();
    for step in &pack.chain_steps {
        // THE invariant: no chain step is ever presented as verified.
        if step.status != NEEDS_REVIEW {
            violations.push(format!(
                "chain step {:?}: status {:?} is forbidden (must be {NEEDS_REVIEW:?})",
                step.id, step.status
            ));
        }
        if !slugs.contains(step.authority.as_str()) {
            violations.push(format!(
                "chain step {:?}: unknown authority {:?}",
                step.id, step.authority
            ));
        }
        if step.validation.trim().is_empty() {
            violations.push(format!("chain step {:?}: missing validation ref", step.id));
        }
        if !orders.insert(step.order) {
            violations.push(format!("duplicate chain order {}", step.order));
        }
        if !step_ids.insert(step.id.as_str()) {
            violations.push(format!("duplicate chain step id {:?}", step.id));
        }
    }

    for (index, observation) in pack.observations.iter().enumerate() {
        // THE invariant, second half: no observed fee/address is usable.
        if observation.status != NEEDS_REVIEW {
            violations.push(format!(
                "observation #{index}: status {:?} is forbidden (must be {NEEDS_REVIEW:?})",
                observation.status
            ));
        }
        if !slugs.contains(observation.authority.as_str()) {
            violations.push(format!(
                "observation #{index}: unknown authority {:?}",
                observation.authority
            ));
        }
        if Topic::parse(&observation.topic).is_err() {
            violations.push(format!(
                "observation #{index}: unknown topic {:?}",
                observation.topic
            ));
        }
        if !is_http_url(&observation.source_uri) {
            violations.push(format!(
                "observation #{index}: source_uri {:?} is not absolute http(s)",
                observation.source_uri
            ));
        }
        if observation.statement.trim().is_empty() {
            violations.push(format!("observation #{index}: empty statement"));
        }
        if observation.validation.trim().is_empty() {
            violations.push(format!("observation #{index}: missing validation ref"));
        }
    }
    violations
}

/// True for absolute `http://` / `https://` URLs with a host.
fn is_http_url(uri: &str) -> bool {
    uri_host(uri).is_some()
}

/// Lowercased host of an absolute `http(s)` URL, if well-formed.
pub(crate) fn uri_host(uri: &str) -> Option<String> {
    let rest = uri
        .strip_prefix("http://")
        .or_else(|| uri.strip_prefix("https://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    // Reject userinfo/ports/empties: packs only need bare hosts.
    if host.is_empty()
        || host.contains('@')
        || host.contains(':')
        || !host.contains('.')
        || host.starts_with('.')
        || host.ends_with('.')
    {
        return None;
    }
    Some(host.to_lowercase())
}

/// Rows written by one [`load_pack`] call (all zero on re-load:
/// loading is idempotent).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadReport {
    /// New `authority` rows.
    pub authorities: usize,
    /// Existing rows whose name, kind, place or v2 fields changed.
    pub updated: usize,
    /// Rows this load hid.
    pub hidden: usize,
    /// New `domain` rows.
    pub domains: usize,
    /// New `source` rows.
    pub sources: usize,
    /// New `task` rows.
    pub tasks: usize,
    /// New `authority_path` rows.
    pub paths: usize,
    /// Open pack tasks closed because the pack no longer has their
    /// step or observation.
    pub closed: usize,
    /// `authority_scope` rows written, changed or dropped.
    pub routes: usize,
}

/// The sweep's `poll_cadence_sec` for a validated pack cadence.
fn cadence_secs(cadence: &str) -> i64 {
    match cadence {
        "hourly" => 3_600,
        "weekly" => 604_800,
        _ => 86_400,
    }
}

/// Task key of one observation: its content, not its position in the
/// file (P3-8: inserting an observation used to shift every later key,
/// skipping the new one and duplicating the shifted one).
fn observation_key(pack: &str, observation: &PackObservation) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for part in [
        observation.authority.as_str(),
        observation.topic.as_str(),
        observation.statement.as_str(),
        observation.source_uri.as_str(),
    ] {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    let digest = hasher.finalize();
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("pack:{pack}:obs:h{hex}")
}

fn slug_id(conn: &Connection, slug: &str) -> Result<Option<i64>, rusqlite::Error> {
    conn.query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |row| {
        row.get(0)
    })
    .optional()
}

/// Load one validated pack into an already-migrated store.
///
/// Writes `authority`, `authority_path`, `domain`, `source`, and
/// `task` rows only — NEVER `fact` or `capture` rows. An authority
/// whose slug exists is brought in line with the pack (name, kind,
/// country, region and the v2 fields it sets), with one
/// `authority_update` audit row carrying before/after; `"hidden"`
/// hides or unhides it (`authority_hide` / `authority_unhide`). A
/// `merges_into` entry writes no row: its links and tasks attach to
/// the target, which must already be loaded (curated packs first).
/// Hidden rows get no new tasks. Re-loading is a no-op. The whole
/// pack commits atomically.
///
/// Callers must run [`validate`] first (via [`validate_dir`]); this
/// function trusts the vocabulary but still lets SQLite CHECKs and
/// FKs reject anything unexpected.
///
/// # Errors
///
/// Returns the store error when any write fails, or when a
/// `merges_into` target is not in the store yet.
pub fn load_pack(
    conn: &Connection,
    file: &PackFile,
    actor: &str,
    now: i64,
) -> contracts::Result<LoadReport> {
    use handbook_store::db::{edit_authority, set_authority_hidden};
    let pack = &file.pack;
    let mut report = LoadReport::default();
    handbook_store::db::in_transaction(conn, || {
        let mut authority_ids: HashMap<&str, i64> = HashMap::new();
        let mut hidden_ids: HashSet<i64> = HashSet::new();
        // Live pack tasks this load still stands behind; the rest are
        // for steps, observations or routes the pack no longer has.
        let mut current_keys: HashSet<String> = HashSet::new();
        for (i, authority) in pack.authorities.iter().enumerate() {
            let ext = file.ext_of(i);
            if let Some(target) = ext.merges_into.as_deref() {
                let id = slug_id(conn, target)?.ok_or_else(|| {
                    contracts::HandbookError::Io(std::io::Error::other(format!(
                        "pack {}: {:?} merges into {target:?}, which is not loaded \
                         (load the curated packs first)",
                        pack.pack, authority.slug
                    )))
                })?;
                authority_ids.insert(authority.slug.as_str(), id);
                load_routes(
                    conn,
                    &pack.pack,
                    id,
                    &ext.scope,
                    (actor, now),
                    &mut report,
                    &mut current_keys,
                )?;
                continue;
            }
            let iso2 = file.iso2_of(i);
            let aliases = ext.aliases.join("\n");
            let v2: Vec<(&str, &str)> = [
                ("iso2", iso2.as_str()),
                ("subdivision", ext.subdivision.as_str()),
                ("level", ext.level.as_str()),
                ("name_en", ext.name_en.as_str()),
                ("aliases", aliases.as_str()),
            ]
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .collect();
            let id = match slug_id(conn, &authority.slug)? {
                Some(id) => {
                    let mut fields = vec![
                        ("kind", authority.kind.as_str()),
                        ("country", authority.country.as_str()),
                        ("region", authority.region.as_str()),
                        ("name", authority.name.as_str()),
                    ];
                    fields.extend(v2.iter().copied());
                    if !edit_authority(conn, id, &fields, actor, now)?.is_empty() {
                        report.updated += 1;
                    }
                    id
                }
                None => {
                    let new = handbook_store::db::NewAuthority {
                        kind: &authority.kind,
                        country: &authority.country,
                        region: &authority.region,
                        name: &authority.name,
                        slug: &authority.slug,
                        actor,
                        at: now,
                    };
                    // Same row shape as `add_authority`, minus its own
                    // transaction: this whole pack already runs in one.
                    let id = add_authority_inner(conn, &new, &v2)?;
                    report.authorities += 1;
                    id
                }
            };
            if let Some(hidden) = ext.hidden {
                let reason = format!("pack {}", pack.pack);
                if set_authority_hidden(conn, id, hidden, actor, now, &reason)? && hidden {
                    report.hidden += 1;
                }
            }
            let is_hidden: bool = conn.query_row(
                "SELECT hidden_at IS NOT NULL FROM authority WHERE id = ?1",
                [id],
                |row| row.get(0),
            )?;
            if is_hidden {
                hidden_ids.insert(id);
            }
            load_routes(
                conn,
                &pack.pack,
                id,
                &ext.scope,
                (actor, now),
                &mut report,
                &mut current_keys,
            )?;
            for (n, path) in ext.paths.iter().enumerate() {
                report.paths += conn.execute(
                    "INSERT OR IGNORE INTO authority_path(path, authority_slug, is_primary,
                                                          created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![path, authority.slug, i64::from(n == 0), now],
                )?;
            }
            authority_ids.insert(authority.slug.as_str(), id);
        }

        let siblings = {
            let mut ids: Vec<String> = pack
                .authorities
                .iter()
                .enumerate()
                .filter(|(i, _)| file.ext_of(*i).merges_into.is_none())
                .filter_map(|(_, a)| authority_ids.get(a.slug.as_str()))
                .map(i64::to_string)
                .collect();
            ids.push(String::from("-1"));
            ids.join(",")
        };

        for domain in &pack.domains {
            let authority_id = authority_ids[domain.authority.as_str()];
            let inserted = conn.execute(
                "INSERT OR IGNORE INTO domain(name, authority_id, status, notes)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    domain.name.to_lowercase(),
                    authority_id,
                    domain.status,
                    domain.notes
                ],
            )?;
            report.domains += inserted;
        }

        for seed in &pack.watch {
            let authority_id = authority_ids[seed.authority.as_str()];
            // `uri_host` was checked by `validate`; fall back to no
            // domain link rather than fail a validated pack.
            let domain_id: Option<String> = uri_host(&seed.uri);
            let exists: Option<i64> = conn
                .query_row(
                    "SELECT id FROM source WHERE authority_id = ?1 AND uri = ?2",
                    rusqlite::params![authority_id, seed.uri],
                    |row| row.get(0),
                )
                .optional()?;
            // A seed this pack used to give a sibling row (positional
            // URL owners before P2-1) moves to its owner while nothing
            // was captured from it; with captures it stays as history
            // and the owner gets its own row.
            let moved = if exists.is_none() {
                conn.execute(
                    &format!(
                        "UPDATE source SET authority_id = ?1
                         WHERE id = (SELECT s.id FROM source s
                                     WHERE s.uri = ?2 AND s.authority_id IN ({siblings})
                                       AND NOT EXISTS (SELECT 1 FROM capture c
                                                       WHERE c.source_id = s.id)
                                     ORDER BY s.id LIMIT 1)"
                    ),
                    rusqlite::params![authority_id, seed.uri],
                )?
            } else {
                0
            };
            if exists.is_none() && moved == 0 {
                let notes = if seed.notes.is_empty() {
                    format!("cadence:{};", seed.cadence)
                } else {
                    format!("cadence:{}; {}", seed.cadence, seed.notes)
                };
                conn.execute(
                    "INSERT INTO source(authority_id, kind, uri, domain_id, notes,
                                        poll_cadence_sec)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        authority_id,
                        seed.kind,
                        seed.uri,
                        domain_id,
                        notes,
                        cadence_secs(&seed.cadence)
                    ],
                )?;
                report.sources += 1;
            }
        }

        for step in &pack.chain_steps {
            let authority_id = authority_ids[step.authority.as_str()];
            if hidden_ids.contains(&authority_id) {
                continue;
            }
            let dedup_key = format!("pack:{}:step:{}", pack.pack, step.id);
            current_keys.insert(dedup_key.clone());
            let title = format!("Chain step needs verification: {}", step.title);
            let detail = if step.detail.is_empty() {
                format!("validation: {}", step.validation)
            } else {
                format!("{}\nvalidation: {}", step.detail, step.validation)
            };
            report.tasks +=
                insert_task_once(conn, authority_id, &title, &detail, &dedup_key, actor, now)?;
        }
        for observation in &pack.observations {
            let authority_id = authority_ids[observation.authority.as_str()];
            if hidden_ids.contains(&authority_id) {
                continue;
            }
            let dedup_key = observation_key(&pack.pack, observation);
            current_keys.insert(dedup_key.clone());
            let title = format!(
                "Verify {} observation ({})",
                observation.authority, observation.topic
            );
            let detail = format!(
                "{}\nsource: {}\nvalidation: {}",
                observation.statement, observation.source_uri, observation.validation
            );
            // A live task from before content keys (`pack:X:obs:<n>`)
            // with this exact text takes the new key (and the
            // observation's current owner) instead of a twin.
            let rekeyed = conn.execute(
                "UPDATE task SET dedup_key = ?1, authority_id = ?4, title = ?5
                 WHERE id = (SELECT id FROM task
                             WHERE dedup_key GLOB ?2 AND detail = ?3
                               AND status IN ('open', 'batched', 'in_progress')
                             ORDER BY id LIMIT 1)
                   AND NOT EXISTS (SELECT 1 FROM task WHERE dedup_key = ?1
                                   AND status IN ('open', 'batched', 'in_progress'))",
                rusqlite::params![
                    dedup_key,
                    format!("pack:{}:obs:[0-9]*", pack.pack),
                    detail,
                    authority_id,
                    title
                ],
            )?;
            if rekeyed == 0 {
                report.tasks +=
                    insert_task_once(conn, authority_id, &title, &detail, &dedup_key, actor, now)?;
            }
        }
        let stale: Vec<i64> = {
            let mut stmt = conn.prepare(
                "SELECT id, dedup_key FROM task
                 WHERE (dedup_key GLOB ?1 OR dedup_key GLOB ?2 OR dedup_key GLOB ?3)
                   AND status IN ('open', 'batched')",
            )?;
            stmt.query_map(
                [
                    format!("pack:{}:obs:*", pack.pack),
                    format!("pack:{}:step:*", pack.pack),
                    format!("pack:{}:scope:*", pack.pack),
                ],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )?
            .filter_map(Result::ok)
            .filter(|(_, key)| !current_keys.contains(key))
            .map(|(id, _)| id)
            .collect()
        };
        for id in stale {
            conn.execute(
                "UPDATE task SET status = 'rejected', resolved_at = ?1,
                        resolution = 'no longer in the pack'
                 WHERE id = ?2",
                [now, id],
            )?;
            report.closed += 1;
        }
        Ok(report)
    })
}

/// Bring one office's `authority_scope` rows from `pack` in line with
/// `routes` (insert, update, drop the ones the pack no longer lists)
/// and keep one open review task for them, keyed on their content.
/// A hidden office gets no task.
fn load_routes(
    conn: &Connection,
    pack: &str,
    authority_id: i64,
    routes: &[ScopeEntry],
    (actor, now): (&str, i64),
    report: &mut LoadReport,
    current_keys: &mut HashSet<String>,
) -> Result<(), rusqlite::Error> {
    for route in routes {
        report.routes += conn.execute(
            "INSERT INTO authority_scope(authority_id, doc_class, issued_in, prereq, source_uri,
                                         pack, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(authority_id, doc_class, issued_in) DO UPDATE
               SET prereq = excluded.prereq, source_uri = excluded.source_uri,
                   pack = excluded.pack
               WHERE prereq != excluded.prereq OR source_uri != excluded.source_uri
                  OR pack != excluded.pack",
            rusqlite::params![
                authority_id,
                route.doc_class,
                route.issued_in,
                route.prereq,
                route.source_uri,
                pack,
                now
            ],
        )?;
    }
    let listed: Vec<(i64, String, String)> = conn
        .prepare(
            "SELECT id, doc_class, issued_in FROM authority_scope
             WHERE authority_id = ?1 AND pack = ?2",
        )?
        .query_map(rusqlite::params![authority_id, pack], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<Result<_, _>>()?;
    for (id, class, issued_in) in listed {
        if !routes
            .iter()
            .any(|r| r.doc_class == class && r.issued_in == issued_in)
        {
            report.routes += conn.execute("DELETE FROM authority_scope WHERE id = ?1", [id])?;
        }
    }
    let (name, slug, hidden): (String, String, bool) = conn.query_row(
        "SELECT name, slug, hidden_at IS NOT NULL FROM authority WHERE id = ?1",
        [authority_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if routes.is_empty() || hidden {
        return Ok(());
    }
    use sha2::{Digest, Sha256};
    let mut lines: Vec<String> = routes
        .iter()
        .map(|r| {
            let place = if r.issued_in.is_empty() {
                String::new()
            } else {
                format!(" issued in {}", r.issued_in)
            };
            let prereq = if r.prereq.is_empty() {
                String::new()
            } else {
                format!("; after: {}", r.prereq)
            };
            format!(
                "doc_rule: {}{place} → {name}{prereq} (source: {})",
                r.doc_class, r.source_uri
            )
        })
        .collect();
    lines.sort();
    let digest = Sha256::digest(lines.join("\n").as_bytes());
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    let key = format!("pack:{pack}:scope:{slug}:h{hex}");
    current_keys.insert(key.clone());
    let detail = format!(
        "{}\nAccept each rule a capture confirms as a jurisdiction fact starting \
         `doc_rule: <doc_class>`; until then place pages show it unverified.\nvalidation: P4-1",
        lines.join("\n")
    );
    report.tasks += insert_task_once(
        conn,
        authority_id,
        &format!("Verify document routing: {name}"),
        &detail,
        &key,
        actor,
        now,
    )?;
    Ok(())
}

/// Insert the authority row plus its audit row without opening a new
/// transaction (the caller already holds one). Mirrors
/// [`handbook_store::db::add_authority`]'s row shape; `v2` columns
/// (`iso2`, `level`, …) are part of the new row, so its one
/// `authority_add` audit row covers them.
fn add_authority_inner(
    conn: &Connection,
    authority: &handbook_store::db::NewAuthority<'_>,
    v2: &[(&str, &str)],
) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO authority(kind, country, region, name, slug)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        (
            authority.kind,
            authority.country,
            authority.region,
            authority.name,
            authority.slug,
        ),
    )?;
    let id = conn.last_insert_rowid();
    for (column, value) in v2 {
        if handbook_store::db::AUTHORITY_EDITABLE.contains(column) {
            conn.execute(
                &format!("UPDATE authority SET {column} = ?1 WHERE id = ?2"),
                rusqlite::params![value, id],
            )?;
        }
    }
    conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, 'authority_add', 'authority', ?3, ?4)",
        rusqlite::params![authority.at, authority.actor, id, authority.slug],
    )?;
    Ok(id)
}

/// Insert one open `validate_fact` task unless a live row with the
/// same `dedup_key` already exists. Returns 1 on insert, 0 on skip.
fn insert_task_once(
    conn: &Connection,
    authority_id: i64,
    title: &str,
    detail: &str,
    dedup_key: &str,
    actor: &str,
    now: i64,
) -> Result<usize, rusqlite::Error> {
    let inserted = conn.execute(
        "INSERT INTO task(authority_id, kind, title, detail, status,
                           priority, dedup_key, created_by, created_at)
         SELECT ?1, 'validate_fact', ?2, ?3, 'open', 0, ?4, ?5, ?6
         WHERE NOT EXISTS (
           SELECT 1 FROM task
           WHERE dedup_key = ?4
             AND status IN ('open', 'batched', 'in_progress')
         )",
        rusqlite::params![authority_id, title, detail, dedup_key, actor, now],
    )?;
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid pack for invariant-probing tests.
    fn minimal_pack() -> (Pack, Vec<u8>) {
        let json = serde_json::json!({
            "pack": "probe",
            "version": 1,
            "title": "probe",
            "authorities": [
                {"kind": "office", "country": "X", "region": "",
                 "name": "Probe Office", "slug": "probe"}
            ],
            "domains": [
                {"name": "example.gov", "authority": "probe",
                 "status": "unverified", "notes": ""}
            ],
            "watch": [
                {"authority": "probe", "kind": "website_official",
                 "uri": "https://example.gov/page", "cadence": "daily",
                 "notes": ""}
            ],
            "chain_steps": [
                {"id": "s", "authority": "probe", "order": 0,
                 "title": "Step", "detail": "", "status": "needs_review",
                 "validation": "V-1"}
            ],
            "observations": [
                {"authority": "probe", "topic": "fees",
                 "statement": "OBSERVED: $1.", "source_uri": "https://example.gov/f",
                 "status": "needs_review", "validation": "V-1"}
            ],
        });
        let raw = serde_json::to_vec(&json).unwrap();
        (serde_json::from_value(json).unwrap(), raw)
    }

    fn rejections(pack: &Pack, raw: &[u8]) -> Vec<String> {
        validate(pack, "probe", raw)
    }

    #[test]
    fn minimal_pack_validates() {
        let (pack, raw) = minimal_pack();
        assert!(rejections(&pack, &raw).is_empty());
    }

    #[test]
    fn chain_step_status_other_than_needs_review_rejected() {
        let (mut pack, _) = minimal_pack();
        pack.chain_steps[0].status = String::from("accepted");
        let raw = serde_json::to_vec(&pack).unwrap_or_default();
        let violations = rejections(&pack, &raw);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(violations[0].contains("needs_review"), "{violations:?}");
    }

    #[test]
    fn observation_status_other_than_needs_review_rejected() {
        let (mut pack, _) = minimal_pack();
        pack.observations[0].status = String::from("proposed");
        let raw = serde_json::to_vec(&pack).unwrap_or_default();
        let violations = rejections(&pack, &raw);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(violations[0].contains("needs_review"), "{violations:?}");
    }

    #[test]
    fn unknown_topic_rejected() {
        let (mut pack, _) = minimal_pack();
        pack.observations[0].topic = String::from("bitcoin");
        let raw = serde_json::to_vec(&pack).unwrap_or_default();
        let violations = rejections(&pack, &raw);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(violations[0].contains("unknown topic"), "{violations:?}");
    }

    #[test]
    fn every_contract_topic_accepted() {
        let (pack, _) = minimal_pack();
        for topic in Topic::ALL {
            let mut pack = pack.clone();
            pack.observations[0].topic = topic.as_str().to_string();
            let raw = serde_json::to_vec(&pack).unwrap_or_default();
            assert!(rejections(&pack, &raw).is_empty(), "{topic:?}");
        }
    }

    #[test]
    fn unwatched_host_without_verdict_rejected() {
        let (mut pack, _) = minimal_pack();
        pack.domains.clear();
        let raw = serde_json::to_vec(&pack).unwrap_or_default();
        let violations = rejections(&pack, &raw);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(violations[0].contains("no verdict row"), "{violations:?}");
    }

    #[test]
    fn fixture_po_boxes_rejected() {
        for banned in BANNED_FIXTURES {
            let (pack, _) = minimal_pack();
            let mut raw = serde_json::to_vec(&pack).unwrap();
            raw.extend_from_slice(format!(" {banned}").as_bytes());
            let violations = rejections(&pack, &raw);
            assert_eq!(violations.len(), 1, "{banned}: {violations:?}");
        }
    }

    #[test]
    fn pack_id_must_match_filename_stem() {
        let (pack, raw) = minimal_pack();
        let violations = validate(&pack, "other", &raw);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(violations[0].contains("filename stem"), "{violations:?}");
    }

    fn scratch_store() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = handbook_store::store::Store::new(dir.path());
        store.init().unwrap();
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        (dir, conn)
    }

    /// A pack file from JSON (both views), as `packs-load` reads it.
    fn file_of(json: &serde_json::Value) -> PackFile {
        PackFile::parse(&serde_json::to_vec(json).unwrap()).unwrap()
    }

    /// Load one pack into a scratch migrated store; return row counts
    /// `(authorities, domains, sources, tasks, facts, captures)`.
    fn load_counts(pack: &Pack) -> (i64, i64, i64, i64, i64, i64) {
        let (_dir, conn) = scratch_store();
        let file = PackFile::v1(pack.clone());
        let report = load_pack(&conn, &file, "test", 1_800_000_000).unwrap();
        assert_eq!(report.authorities, pack.authorities.len());
        // Re-load must be a no-op (idempotent).
        let again = load_pack(&conn, &file, "test", 1_800_000_000).unwrap();
        assert_eq!(again, LoadReport::default());
        let count = |table: &str| -> i64 {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
        };
        (
            count("authority"),
            count("domain"),
            count("source"),
            count("task"),
            count("fact"),
            count("capture"),
        )
    }

    #[test]
    fn load_sets_the_sweep_cadence_from_the_pack() {
        let (mut pack, _raw) = minimal_pack();
        pack.watch[0].cadence = "weekly".to_string();
        let (_dir, conn) = scratch_store();
        load_pack(&conn, &PackFile::v1(pack), "test", 1_800_000_000).unwrap();
        let secs: i64 = conn
            .query_row("SELECT poll_cadence_sec FROM source", [], |row| row.get(0))
            .unwrap();
        assert_eq!(secs, 604_800);
    }

    fn row(conn: &Connection, slug: &str) -> (String, String, String, Option<i64>, String) {
        conn.query_row(
            "SELECT name, kind, region, hidden_at, iso2 FROM authority WHERE slug = ?1",
            [slug],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap()
    }

    /// P2-2: a corrected pack reaches a store that already holds the
    /// row: name/kind/region follow the pack, with one audited update;
    /// `"hidden": true` hides it; reloading changes nothing.
    #[test]
    fn a_corrected_pack_updates_the_existing_row_with_audit() {
        let (_dir, conn) = scratch_store();
        let shipped = serde_json::json!({
            "pack": "DE", "version": 1, "title": "t",
            "authorities": [
                {"kind": "country", "country": "Germany", "region": "",
                 "name": "Land", "slug": "de-ca-1"},
                {"kind": "country", "country": "Germany", "region": "",
                 "name": "Oberlandesgericht", "slug": "de-ca-2"}
            ],
            "domains": [
                {"name": "dpma.de", "authority": "de-ca-2", "status": "unverified", "notes": ""},
                {"name": "bfaa.diplo.de", "authority": "de-ca-2", "status": "unverified",
                 "notes": ""}
            ],
            // Positional owners, as world-index wrote them before P2-1.
            "watch": [
                {"authority": "de-ca-2", "kind": "website_official",
                 "uri": "https://dpma.de/x", "cadence": "weekly", "notes": ""},
                {"authority": "de-ca-2", "kind": "website_official",
                 "uri": "https://bfaa.diplo.de/x", "cadence": "weekly", "notes": ""}
            ],
            "observations": [
                {"authority": "de-ca-2", "topic": "contacts",
                 "statement": "OBSERVED: designated authority: Oberlandesgericht.",
                 "source_uri": "https://www.hcch.net/x", "status": "needs_review",
                 "validation": "W-7"}
            ]
        });
        load_pack(&conn, &file_of(&shipped), "test", 1).unwrap();
        // One of the two seeds was already captured under its old owner.
        conn.execute(
            "INSERT INTO capture(authority_id, source_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             SELECT authority_id, id, 5, 'warc', 'x', 1, 1, 'h', 'official'
             FROM source WHERE uri = 'https://bfaa.diplo.de/x'",
            [],
        )
        .unwrap();
        let tasks_open = |conn: &Connection| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM task WHERE status = 'open'", [], |r| {
                r.get(0)
            })
            .unwrap()
        };
        assert_eq!(tasks_open(&conn), 1);

        let mut corrected = shipped.clone();
        corrected["iso2"] = serde_json::json!("DE");
        corrected["authorities"][0]["name"] =
            serde_json::json!("Germany — Competent Authority (Art. 6)");
        corrected["authorities"][0]["source_name"] = serde_json::json!("Land");
        corrected["authorities"][1]["hidden"] = serde_json::json!(true);
        for seed in corrected["watch"].as_array_mut().unwrap() {
            seed["authority"] = serde_json::json!("de-ca-1");
        }
        for domain in corrected["domains"].as_array_mut().unwrap() {
            domain["authority"] = serde_json::json!("de-ca-1");
        }
        let file = file_of(&corrected);
        assert!(validate(&file.pack, "DE", &serde_json::to_vec(&corrected).unwrap()).is_empty());
        let report = load_pack(&conn, &file, "pack-loader", 2).unwrap();
        assert_eq!(
            (
                report.authorities,
                report.updated,
                report.hidden,
                report.sources
            ),
            (0, 2, 1, 1),
            "{report:?}"
        );
        // The uncaptured seed moved to its owner; the captured one
        // stays as history beside a fresh row for the owner.
        let owners: Vec<(String, String)> = conn
            .prepare(
                "SELECT s.uri, a.slug FROM source s JOIN authority a ON a.id = s.authority_id
                 ORDER BY s.uri, s.id",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            owners,
            [
                ("https://bfaa.diplo.de/x".to_string(), "de-ca-2".to_string()),
                ("https://bfaa.diplo.de/x".to_string(), "de-ca-1".to_string()),
                ("https://dpma.de/x".to_string(), "de-ca-1".to_string()),
            ]
        );
        assert_eq!(
            row(&conn, "de-ca-1"),
            (
                "Germany — Competent Authority (Art. 6)".to_string(),
                "country".to_string(),
                String::new(),
                None,
                "DE".to_string()
            )
        );
        assert_eq!(row(&conn, "de-ca-2").3, Some(2));
        // The hidden row's open pack task is closed, not left for staff.
        assert_eq!(tasks_open(&conn), 0);
        let (before, after, actor): (String, String, String) = conn
            .query_row(
                "SELECT before_ref, after_ref, actor FROM audit
                 WHERE action = 'authority_update' AND entity_id =
                       (SELECT id FROM authority WHERE slug = 'de-ca-1')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(before, "name=Land\niso2=");
        assert_eq!(
            after,
            "name=Germany — Competent Authority (Art. 6)\niso2=DE"
        );
        assert_eq!(actor, "pack-loader");
        let hides: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'authority_hide'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hides, 1);
        // Idempotent.
        assert_eq!(
            load_pack(&conn, &file, "pack-loader", 3).unwrap(),
            LoadReport::default()
        );
        // A pack without "hidden" leaves a hidden row hidden.
        load_pack(&conn, &file_of(&shipped), "test", 4).unwrap();
        assert_eq!(row(&conn, "de-ca-2").3, Some(2));
    }

    #[test]
    fn merges_into_attaches_to_the_target_and_writes_no_row() {
        let (_dir, conn) = scratch_store();
        let world = serde_json::json!({
            "pack": "US", "version": 1, "title": "t",
            "authorities": [
                {"kind": "state", "country": "United States of America",
                 "region": "Washington", "name": "Washington Secretary of State",
                 "slug": "us-wa-sos", "merges_into": "usa-wa-sos"}
            ],
            "domains": [{"name": "sos.wa.gov", "authority": "us-wa-sos",
                         "status": "unverified", "notes": ""}],
            "watch": [{"authority": "us-wa-sos", "kind": "website_official",
                       "uri": "https://sos.wa.gov/x", "cadence": "weekly", "notes": ""}]
        });
        let err = load_pack(&conn, &file_of(&world), "test", 1).unwrap_err();
        assert!(err.to_string().contains("not loaded"), "{err}");
        let curated = serde_json::json!({
            "pack": "usa-wa", "version": 1, "title": "t", "iso2": "US",
            "authorities": [
                {"kind": "state", "country": "USA", "region": "Washington",
                 "name": "Washington Secretary of State — Apostilles", "slug": "usa-wa-sos",
                 "subdivision": "US-WA", "level": "state",
                 "paths": ["usa/washington"]}
            ]
        });
        load_pack(&conn, &file_of(&curated), "test", 1).unwrap();
        let report = load_pack(&conn, &file_of(&world), "test", 2).unwrap();
        assert_eq!(
            (report.authorities, report.updated, report.sources),
            (0, 0, 1)
        );
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM authority WHERE id <> 0", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(rows, 1);
        // The curated name survives; the world link hangs off its row.
        assert_eq!(
            row(&conn, "usa-wa-sos").0,
            "Washington Secretary of State — Apostilles"
        );
        let owner: String = conn
            .query_row(
                "SELECT a.slug FROM source s JOIN authority a ON a.id = s.authority_id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(owner, "usa-wa-sos");
        let (level, subdivision, path): (String, String, String) = conn
            .query_row(
                "SELECT a.level, a.subdivision, p.path FROM authority a
                 JOIN authority_path p ON p.authority_slug = a.slug AND p.is_primary = 1
                 WHERE a.slug = 'usa-wa-sos'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (level.as_str(), subdivision.as_str(), path.as_str()),
            ("state", "US-WA", "usa/washington")
        );
    }

    /// P4-1: routes load as `authority_scope` rows plus one review
    /// task per office, never facts; a merging entry's routes attach to
    /// the target; a changed route list replaces the rows and the task.
    #[test]
    fn routes_load_as_scope_rows_and_one_task_never_facts() {
        let (_dir, conn) = scratch_store();
        let src = "https://www.hcch.net/en/states/authorities/details3/?aid=353";
        let pack = |classes: &[&str]| {
            let scope: Vec<serde_json::Value> = classes
                .iter()
                .map(|c| serde_json::json!({"doc_class": c, "issued_in": "US-WA", "source_uri": src}))
                .collect();
            serde_json::json!({
                "pack": "US", "version": 1, "title": "t", "iso2": "US",
                "authorities": [
                    {"kind": "state", "country": "USA", "region": "Washington",
                     "name": "Washington Secretary of State", "slug": "us-wa-sos",
                     "subdivision": "US-WA", "level": "state", "scope": scope}
                ]
            })
        };
        let file = file_of(&pack(&["vital_record", "state_court"]));
        let raw = serde_json::to_vec(&pack(&["vital_record", "state_court"])).unwrap();
        assert!(validate(&file.pack, "US", &raw).is_empty());
        let report = load_pack(&conn, &file, "test", 1).unwrap();
        assert_eq!((report.routes, report.tasks), (2, 1));
        assert_eq!(
            load_pack(&conn, &file, "test", 2).unwrap(),
            LoadReport::default()
        );
        let report = load_pack(&conn, &file_of(&pack(&["vital_record"])), "test", 3).unwrap();
        assert_eq!((report.routes, report.tasks, report.closed), (1, 1, 1));
        let rows: Vec<(String, String)> = conn
            .prepare("SELECT doc_class, issued_in FROM authority_scope")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(rows, [("vital_record".to_string(), "US-WA".to_string())]);
        let facts: i64 = conn
            .query_row("SELECT COUNT(*) FROM fact", [], |r| r.get(0))
            .unwrap();
        assert_eq!(facts, 0);
        let bad = serde_json::json!({
            "pack": "US", "version": 1,
            "authorities": [{"kind": "office", "country": "X", "region": "",
                             "name": "Probe office", "slug": "probe",
                             "scope": [{"doc_class": "pets", "issued_in": "usa",
                                        "source_uri": "ftp://x"}]}]
        });
        let raw = serde_json::to_vec(&bad).unwrap();
        let violations = validate(&file_of(&bad).pack, "US", &raw);
        for needle in ["doc_class", "issued_in", "source_uri"] {
            assert!(
                violations.iter().any(|v| v.contains(needle)),
                "{needle}: {violations:?}"
            );
        }
    }

    /// P3-8: inserting an observation adds exactly one task; nothing
    /// shifts or duplicates. A live task under the old positional key
    /// is re-keyed, not twinned.
    #[test]
    fn observation_tasks_key_on_content_not_position() {
        let (_dir, conn) = scratch_store();
        let obs = |n: u32| {
            serde_json::json!({
                "authority": "probe", "topic": "fees",
                "statement": format!("OBSERVED: ${n}."), "source_uri": "https://example.gov/f",
                "status": "needs_review", "validation": "V-1"})
        };
        let pack = |list: Vec<serde_json::Value>| {
            serde_json::json!({
                "pack": "probe", "version": 1, "title": "t",
                "authorities": [{"kind": "office", "country": "X", "region": "",
                                 "name": "Probe office", "slug": "probe"}],
                "observations": list
            })
        };
        // A task as the positional loader left it for observation #0.
        load_pack(&conn, &file_of(&pack(vec![])), "test", 1).unwrap();
        conn.execute(
            "INSERT INTO task(authority_id, kind, title, detail, status, dedup_key, created_at)
             SELECT id, 'validate_fact', 't',
                    'OBSERVED: $1.\nsource: https://example.gov/f\nvalidation: V-1',
                    'open', 'pack:probe:obs:0', 1
             FROM authority WHERE slug = 'probe'",
            [],
        )
        .unwrap();
        let first = load_pack(&conn, &file_of(&pack(vec![obs(1), obs(2)])), "test", 2).unwrap();
        assert_eq!(first.tasks, 1, "#0 re-keyed, #1 new");
        let second = load_pack(
            &conn,
            &file_of(&pack(vec![obs(3), obs(1), obs(2)])),
            "test",
            3,
        )
        .unwrap();
        assert_eq!(second.tasks, 1, "only the inserted observation");
        let keys: Vec<String> = conn
            .prepare("SELECT dedup_key FROM task ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(keys.len(), 3);
        assert!(
            keys.iter().all(|k| k.starts_with("pack:probe:obs:h")),
            "{keys:?}"
        );
    }

    #[test]
    fn name_lint_flags_page_fragments_and_passes_offices() {
        for junk in [
            "Land",
            "the federal government,",
            "Declaration of 2 August 2004:",
            "Limmatquai 50",
            "CH-8001 Zurich",
            "[email protected]",
            "Telephone: +41-44-268-2211",
            "Fax: +41-44-268-2212",
            "a notary,",
        ] {
            assert!(name_lint(junk).is_some(), "{junk:?} should be flagged");
        }
        for office in [
            "Ministry of Foreign Affairs",
            "Notaries",
            "Tribunals",
            "IRI Corporate and Maritime Services (Switzerland) A.G.",
            "Registrars of Magistrates' Courts and Civil Servants appointed by the Minister \
             of Justice under Notaries Law, 1976",
            "(Drejtoria Konsullore, Zyra e Legalizimeve, Ministria e Puneve te Jashtme)",
            "U.S. Department of State — Office of Authentications",
        ] {
            assert_eq!(name_lint(office), None, "{office:?}");
        }
    }

    #[test]
    fn junk_names_fail_validation_unless_hidden() {
        let json = |hidden: bool| {
            let mut a = serde_json::json!({"kind": "country", "country": "X", "region": "",
                                           "name": "Fax: +41-44-268-2212", "slug": "probe"});
            if hidden {
                a["hidden"] = serde_json::json!(true);
            }
            serde_json::json!({"pack": "probe", "version": 1, "authorities": [a]})
        };
        for (hidden, bad) in [(false, 1), (true, 0)] {
            let raw = serde_json::to_vec(&json(hidden)).unwrap();
            let file = PackFile::parse(&raw).unwrap();
            let violations = validate(&file.pack, "probe", &raw);
            assert_eq!(violations.len(), bad, "{violations:?}");
        }
    }

    #[test]
    fn v2_fields_are_checked() {
        let raw = serde_json::to_vec(&serde_json::json!({
            "pack": "probe", "version": 1, "iso2": "us",
            "authorities": [{"kind": "office", "country": "X", "region": "",
                             "name": "Probe office", "slug": "probe",
                             "subdivision": "DE-BY", "level": "galactic",
                             "paths": ["usa/New York"], "merges_into": "probe"}]
        }))
        .unwrap();
        let file = PackFile::parse(&raw).unwrap();
        let violations = validate(&file.pack, "probe", &raw);
        for needle in ["iso2", "subdivision", "bad level", "path", "merges_into"] {
            assert!(
                violations.iter().any(|v| v.contains(needle)),
                "{needle}: {violations:?}"
            );
        }
    }

    #[test]
    fn a_slug_has_one_owner_across_the_tree() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("world")).unwrap();
        let write = |path: &str, json: serde_json::Value| {
            std::fs::write(root.path().join(path), serde_json::to_vec(&json).unwrap()).unwrap();
        };
        let authority = |slug: &str, extra: serde_json::Value| {
            let mut a = serde_json::json!({"kind": "office", "country": "X", "region": "",
                                           "name": "Probe office", "slug": slug});
            if let Some(map) = extra.as_object() {
                for (k, v) in map {
                    a[k] = v.clone();
                }
            }
            a
        };
        write(
            "a.json",
            serde_json::json!({"pack": "a", "version": 1,
                               "authorities": [authority("shared", serde_json::json!({}))]}),
        );
        write(
            "world/b.json",
            serde_json::json!({"pack": "b", "version": 1,
                               "authorities": [authority("shared", serde_json::json!({}))]}),
        );
        let errors = validate_tree(root.path());
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].to_string().contains("one owner"), "{errors:?}");
        write(
            "world/b.json",
            serde_json::json!({"pack": "b", "version": 1, "authorities": [
                authority("b-shared", serde_json::json!({"merges_into": "shared"})),
                authority("b-ghost", serde_json::json!({"merges_into": "nowhere"}))
            ]}),
        );
        let errors = validate_tree(root.path());
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].to_string().contains("no pack declares"),
            "{errors:?}"
        );
        // One directory alone cannot see the targets, so it does not judge them.
        assert!(validate_dir(&root.path().join("world")).is_empty());
    }

    #[test]
    fn rendered_pack_round_trips_both_views() {
        let raw = serde_json::to_vec(&serde_json::json!({
            "pack": "DE", "version": 1, "iso2": "DE", "title": "t",
            "authorities": [{"kind": "country", "country": "Germany", "region": "",
                             "name": "Land", "slug": "de-ca-1", "hidden": true,
                             "aliases": ["Deutschland"]}]
        }))
        .unwrap();
        let file = PackFile::parse(&raw).unwrap();
        let again = PackFile::parse(file.render().as_bytes()).unwrap();
        assert_eq!(again.ext, file.ext);
        assert_eq!(again.pack.authorities[0].name, "Land");
        assert!(
            file.render()
                .contains("\"slug\": \"de-ca-1\",\n      \"hidden\": true")
        );
    }

    /// The committed tree as `packs-validate` and the deploy gate see
    /// it: every pack set, the name lint, one owner per slug.
    #[test]
    fn committed_pack_tree_validates_clean() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs");
        let errors = validate_tree(&root);
        assert!(errors.is_empty(), "{errors:#?}");
    }

    /// Curated + world packs in load order (curated first, as admin
    /// "Load packs" and ops-packs-seed.sh do) never mint two rows for
    /// one US jurisdiction: world entries for a curated dossier say
    /// `merges_into` and attach to it.
    #[test]
    fn world_rows_merge_into_curated_dossiers() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs");
        let (_dir, conn) = scratch_store();
        let mut rows_declared = 0;
        let mut merging = 0;
        for sub in [root.clone(), root.join("world")] {
            for path in pack_paths(&sub).unwrap() {
                let file = PackFile::parse(&std::fs::read(&path).unwrap()).unwrap();
                for i in 0..file.pack.authorities.len() {
                    if file.ext_of(i).merges_into.is_some() {
                        merging += 1;
                    } else {
                        rows_declared += 1;
                    }
                }
                load_pack(&conn, &file, "test", 1_800_000_000).unwrap();
            }
        }
        assert!(merging >= 6, "the curated US dossiers are merged into");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM authority WHERE id <> 0", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(rows, rows_declared);
        let dupes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM (
                   SELECT region FROM authority
                   WHERE kind = 'state' AND region <> '' AND iso2 = 'US'
                   GROUP BY region HAVING COUNT(*) > 1)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dupes, 0, "one row per US state");
        // Every public row passes the name lint.
        let names: Vec<String> = conn
            .prepare("SELECT name FROM authority WHERE id <> 0 AND hidden_at IS NULL")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let junk: Vec<&String> = names.iter().filter(|n| name_lint(n).is_some()).collect();
        assert!(junk.is_empty(), "{junk:?}");
    }

    #[test]
    fn load_writes_rows_but_never_facts_or_captures() {
        let (pack, raw) = minimal_pack();
        assert!(rejections(&pack, &raw).is_empty());
        // authority count includes the migration-005 sentinel row;
        // capture count includes its sentinel capture.
        let (authorities, domains, sources, tasks, facts, captures) = load_counts(&pack);
        assert_eq!(authorities, 1 + 1);
        assert_eq!(domains, 1);
        assert_eq!(sources, 1);
        assert_eq!(tasks, 2); // one chain step + one observation
        assert_eq!(facts, 0);
        assert_eq!(captures, 1); // sentinel only
    }
}
