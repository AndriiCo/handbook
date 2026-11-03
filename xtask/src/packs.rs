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

/// Read and validate every `*.json` pack in `dir`.
///
/// # Errors
///
/// Returns one [`PackError`] per bad file (bad JSON or a rule
/// violation). An empty vec means every pack is clean.
#[must_use]
pub fn validate_dir(dir: &Path) -> Vec<PackError> {
    let mut errors = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            return vec![PackError {
                pack: dir.display().to_string(),
                violations: vec![format!("cannot list packs dir: {e}")],
            }];
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
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
        match serde_json::from_slice::<Pack>(&bytes) {
            Ok(pack) => {
                let violations = validate(&pack, &stem, &bytes);
                if !violations.is_empty() {
                    errors.push(PackError {
                        pack: pack.pack.clone(),
                        violations,
                    });
                }
            }
            Err(e) => errors.push(PackError {
                pack: stem,
                violations: vec![format!("invalid JSON in {}: {e}", path.display())],
            }),
        }
    }
    errors
}

/// Validate one parsed pack against every pack rule.
///
/// `stem` is the filename stem (must equal `pack.pack`); `raw` is
/// the file bytes (scanned for banned fixture values).
fn validate(pack: &Pack, stem: &str, raw: &[u8]) -> Vec<String> {
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

    let mut slugs = HashSet::new();
    for authority in &pack.authorities {
        if !matches!(
            authority.kind.as_str(),
            "country" | "state" | "province" | "office"
        ) {
            violations.push(format!(
                "authority {:?}: bad kind {:?}",
                authority.slug, authority.kind
            ));
        }
        if authority.slug.trim().is_empty() || authority.name.trim().is_empty() {
            violations.push(String::from("authority slug and name must be non-empty"));
        }
        if !slugs.insert(authority.slug.as_str()) {
            violations.push(format!("duplicate authority slug {:?}", authority.slug));
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
fn uri_host(uri: &str) -> Option<String> {
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
    /// New `domain` rows.
    pub domains: usize,
    /// New `source` rows.
    pub sources: usize,
    /// New `task` rows.
    pub tasks: usize,
}

/// The sweep's `poll_cadence_sec` for a validated pack cadence.
fn cadence_secs(cadence: &str) -> i64 {
    match cadence {
        "hourly" => 3_600,
        "weekly" => 604_800,
        _ => 86_400,
    }
}

/// Load one validated pack into an already-migrated store.
///
/// Writes `authority`, `domain`, `source`, and `task` rows only —
/// NEVER `fact` or `capture` rows. Re-loading is a no-op (existing
/// slugs/URIs/dedup keys are skipped, not duplicated). The whole
/// pack commits atomically.
///
/// Callers must run [`validate`] first (via [`validate_dir`]); this
/// function trusts the vocabulary but still lets SQLite CHECKs and
/// FKs reject anything unexpected.
///
/// # Errors
///
/// Returns the store error when any insert fails.
pub fn load_pack(
    conn: &Connection,
    pack: &Pack,
    actor: &str,
    now: i64,
) -> contracts::Result<LoadReport> {
    let mut report = LoadReport::default();
    handbook_store::db::in_transaction(conn, || {
        let mut authority_ids: HashMap<&str, i64> = HashMap::new();
        for authority in &pack.authorities {
            let existing: Option<i64> = conn
                .query_row(
                    "SELECT id FROM authority WHERE slug = ?1",
                    [&authority.slug],
                    |row| row.get(0),
                )
                .optional()?;
            let id = match existing {
                Some(id) => id,
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
                    let id = add_authority_inner(conn, &new)?;
                    report.authorities += 1;
                    id
                }
            };
            authority_ids.insert(authority.slug.as_str(), id);
        }

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
            if exists.is_none() {
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
            let dedup_key = format!("pack:{}:step:{}", pack.pack, step.id);
            let title = format!("Chain step needs verification: {}", step.title);
            let detail = if step.detail.is_empty() {
                format!("validation: {}", step.validation)
            } else {
                format!("{}\nvalidation: {}", step.detail, step.validation)
            };
            report.tasks +=
                insert_task_once(conn, authority_id, &title, &detail, &dedup_key, actor, now)?;
        }
        for (index, observation) in pack.observations.iter().enumerate() {
            let authority_id = authority_ids[observation.authority.as_str()];
            let dedup_key = format!("pack:{}:obs:{index}", pack.pack);
            let title = format!(
                "Verify {} observation ({})",
                observation.authority, observation.topic
            );
            let detail = format!(
                "{}\nsource: {}\nvalidation: {}",
                observation.statement, observation.source_uri, observation.validation
            );
            report.tasks +=
                insert_task_once(conn, authority_id, &title, &detail, &dedup_key, actor, now)?;
        }
        Ok(report)
    })
}

/// Insert the authority row plus its audit row without opening a new
/// transaction (the caller already holds one). Mirrors
/// [`handbook_store::db::add_authority`]'s row shape.
fn add_authority_inner(
    conn: &Connection,
    authority: &handbook_store::db::NewAuthority<'_>,
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
                 "name": "Probe", "slug": "probe"}
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

    /// Load one pack into a scratch migrated store; return row counts
    /// `(authorities, domains, sources, tasks, facts, captures)`.
    fn load_counts(pack: &Pack) -> (i64, i64, i64, i64, i64, i64) {
        let dir = tempfile::tempdir().unwrap();
        let store = handbook_store::store::Store::new(dir.path());
        store.init().unwrap();
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        let report = load_pack(&conn, pack, "test", 1_800_000_000).unwrap();
        assert_eq!(report.authorities, pack.authorities.len());
        // Re-load must be a no-op (idempotent).
        let again = load_pack(&conn, pack, "test", 1_800_000_000).unwrap();
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
        let dir = tempfile::tempdir().unwrap();
        let store = handbook_store::store::Store::new(dir.path());
        store.init().unwrap();
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        load_pack(&conn, &pack, "test", 1_800_000_000).unwrap();
        let secs: i64 = conn
            .query_row("SELECT poll_cadence_sec FROM source", [], |row| row.get(0))
            .unwrap();
        assert_eq!(secs, 604_800);
    }

    #[test]
    fn committed_world_packs_validate_clean() {
        for sub in ["world", "world-nonparty"] {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../packs")
                .join(sub);
            if dir.is_dir() {
                let errors = validate_dir(&dir);
                assert!(errors.is_empty(), "{sub}: {errors:?}");
            }
        }
    }

    /// Curated + world packs in load order (curated first, as admin
    /// "Load packs" and ops-packs-seed.sh do) never mint two rows for
    /// one US jurisdiction: world rows for a curated dossier carry the
    /// curated slug and merge into it.
    #[test]
    fn world_rows_merge_into_curated_dossiers() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs");
        let dir = tempfile::tempdir().unwrap();
        let store = handbook_store::store::Store::new(dir.path());
        store.init().unwrap();
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        let mut slugs = 0;
        for sub in [root.clone(), root.join("world")] {
            let mut files: Vec<_> = std::fs::read_dir(&sub)
                .unwrap()
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect();
            files.sort();
            for path in files {
                let pack: Pack = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                slugs += pack.authorities.len();
                load_pack(&conn, &pack, "test", 1_800_000_000).unwrap();
            }
        }
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM authority WHERE id <> 0", [], |r| {
                r.get(0)
            })
            .unwrap();
        // Exactly the six curated US dossiers are shared.
        assert_eq!(rows, i64::try_from(slugs).unwrap() - 6);
        let dupes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM (
                   SELECT region FROM authority
                   WHERE kind = 'state' AND region <> ''
                     AND country IN ('USA', 'United States of America')
                   GROUP BY region HAVING COUNT(*) > 1)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dupes, 0, "one row per US state");
    }

    #[test]
    fn committed_packs_validate_clean() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs");
        let errors = validate_dir(&dir);
        assert!(errors.is_empty(), "{errors:?}");
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
