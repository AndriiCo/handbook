//! `discovery.bootstrap`: from-scratch discovery, first slice.
//!
//! The atlas pack `crates/atlas/packs/discovery-bootstrap.toml` owns
//! the model, prompts and schemas; this file owns the orchestration.
//! Nothing here writes to the store or to packs: the result is a report
//! for staff, and every value in it still needs review.
//!
//! 1. **Countries, no model.** UN member states (recorded page) plus the
//!    two observer states, joined with the HCCH 1961 status table (live
//!    by default, the recorded copy with `"recorded": true`). The EIF
//!    column splits parties: `party` (in force today), `future_party`
//!    (joined, entry into force still ahead: legalization applies until
//!    then) and `nonparty`. A status-table name this build cannot map to
//!    an ISO code is reported, never dropped: it is usually a new party.
//! 2. **Cross-check (`hague-crosscheck`).** Muse Spark lists the parties
//!    and their EIF dates from memory; the answer is diffed against the
//!    table (missing, extra, date mismatches, status-table URL). The
//!    model's answer is never used as data.
//! 3. **Official URLs (`find-urls`), batched.** For the world-pack
//!    authorities of the countries in `payload.countries`, Spark is asked
//!    for official URLs `batch_size` authorities at a time. Names that are
//!    page fragments (address, contact or heading lines) are skipped and
//!    listed. Every proposed URL is fetched; it is kept only when it
//!    answers over https and its text names the authority or an
//!    apostille term. Kept URLs carry an `official_hint` (government-style
//!    host, or a host the HCCH pages list for that authority).
//!
//! Payload: `{recorded?: bool, crosscheck?: bool (default true),
//! countries?: [ISO2] | "all", batch_size?: 5..25 (15),
//! max_batches?: n (10)}`. Without `countries` the URL stage is skipped.
//!
//! Every fetched page lands in `<job>/pages/`, every model call's input
//! and output in `<job>/calls/` plus an atlas ledger.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use atlas::config::{AtlasConfig, ModelSpec, TaskSpec};
use atlas::ledger::{CallRecord, Ledger};
use atlas::model::{ChatRequest, JsonSchema, ModelPort};
use serde_json::{Value, json};

use super::research_nonparty::{
    Fetch, LiveFetch, OBSERVERS, fold, host_of, model_json, page_text, un_iso2, un_members,
};
use super::{JobContext, JobError, JobHandler, JobOutput};
use crate::packs::Pack;

/// The atlas pack (repo-relative).
pub const PACK_PATH: &str = "crates/atlas/packs/discovery-bootstrap.toml";
/// Fewer parsed parties than this means the status page changed shape.
const MIN_PARTIES: usize = 100;
const DEFAULT_BATCH: usize = 15;
const DEFAULT_MAX_BATCHES: usize = 10;
/// URLs kept per authority (the schema also caps the model at 3).
const URLS_PER_AUTHORITY: usize = 3;
/// Folded words that mark an apostille or legalization page (Latin,
/// Cyrillic, Greek, Arabic, CJK). Matched as substrings.
const APOSTILLE_TERMS: [&str; 11] = [
    "apostil",
    "апостил",
    "legaliz",
    "legalis",
    "beglaubig",
    "επισημείωση",
    "أبوستيل",
    "アポスティーユ",
    "附加证明书",
    "海牙认证",
    "아포스티유",
];
/// Host labels that mark a government domain (`*.gov.*`, `*.gouv.fr`,
/// `*.gv.at`, `*.go.jp`, `*.admin.ch`, `*.diplo.de`, …). A hint only.
const GOV_LABELS: [&str; 11] = [
    "gov", "gob", "gouv", "gv", "govt", "go", "gub", "gc", "admin", "bund", "diplo",
];

pub struct DiscoveryBootstrap;

impl JobHandler for DiscoveryBootstrap {
    fn kind(&self) -> &'static str {
        "discovery.bootstrap"
    }

    fn run(&self, ctx: &JobContext, payload: &Value) -> Result<JobOutput, JobError> {
        let Some(key) = ctx.env.get("META_API_KEY") else {
            return Err(JobError::fatal(
                "discovery.bootstrap needs META_API_KEY in worker.env",
            ));
        };
        let base = ctx
            .env
            .get("META_BASE_URL")
            .unwrap_or_else(|| atlas::model::BASE_URL.to_string());
        // Same caches and daily cap as research.nonparty (P391): reruns
        // replay saved answers and pages instead of paying again.
        let mode = |var: &str| {
            ctx.env
                .get(var)
                .and_then(|m| callcache::CacheMode::parse(&m))
                .unwrap_or_default()
        };
        let cap = ctx
            .env
            .get("META_DAILY_CALL_CAP")
            .and_then(|c| c.parse().ok())
            .unwrap_or(400);
        let meta =
            callcache::MetaCache::new(ctx.cache_dir.join("meta-cache"), mode("META_CACHE_MODE"))
                .with_budget(cap);
        let port = atlas::model::MetaClient::new(
            atlas::cached::CachedTransport::new(
                atlas::model::UreqTransport::default(),
                std::sync::Arc::new(meta),
            ),
            &base,
            key,
        );
        let fetch = LiveFetch::new(callcache::FetchCache::new(
            ctx.cache_dir.join("fetch-cache"),
            mode("FETCH_CACHE_MODE"),
        ));
        bootstrap(
            &port,
            &fetch,
            &ctx.repo,
            &ctx.dir,
            payload,
            crate::unix_now(),
        )
    }
}

// ---------------------------------------------------------------------------
// Stage 1: countries
// ---------------------------------------------------------------------------

/// One country of the universe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CountryRow {
    /// `None` for a status-table name this build cannot map.
    pub iso2: Option<String>,
    pub name: String,
    /// `party | future_party | nonparty`.
    pub status: &'static str,
    /// Entry into force (`YYYY-MM-DD`) for parties.
    pub eif: Option<String>,
}

/// UN members + observers joined with the status table, by ISO code.
/// `today` is `YYYY-MM-DD`; an EIF after it makes a `future_party`.
pub(crate) fn countries(
    un_html: &str,
    hcch_html: &str,
    today: &str,
) -> Result<Vec<CountryRow>, String> {
    let members = un_members(un_html);
    if members.len() != 193 {
        return Err(format!("UN page lists {} members, want 193", members.len()));
    }
    let parties = crate::world::party_entry_into_force(hcch_html);
    if parties.len() < MIN_PARTIES {
        return Err(format!(
            "HCCH status table lists {} parties (want at least {MIN_PARTIES}); \
             the page may have changed shape",
            parties.len()
        ));
    }
    let mut rows = Vec::new();
    let mut party_codes = HashSet::new();
    for (name, eif) in parties {
        let iso2 = crate::world::party_iso2(&name).map(str::to_string);
        if let Some(code) = &iso2 {
            party_codes.insert(code.clone());
        }
        let status = match &eif {
            Some(date) if date.as_str() > today => "future_party",
            _ => "party",
        };
        rows.push(CountryRow {
            iso2,
            name,
            status,
            eif,
        });
    }
    let others = members
        .iter()
        .filter_map(|name| Some((name.as_str(), un_iso2(name)?)))
        .chain(OBSERVERS);
    for (name, code) in others {
        if !party_codes.contains(code) {
            rows.push(CountryRow {
                iso2: Some(code.to_string()),
                name: name.to_string(),
                status: "nonparty",
                eif: None,
            });
        }
    }
    Ok(rows)
}

fn country_json(row: &CountryRow) -> Value {
    json!({"iso2": row.iso2, "name": row.name, "status": row.status, "entry_into_force": row.eif})
}

// ---------------------------------------------------------------------------
// Stage 2: cross-check
// ---------------------------------------------------------------------------

/// Diff the model's from-memory party list against the table.
pub(crate) fn crosscheck_diff(answer: &Value, rows: &[CountryRow]) -> Value {
    let url = answer["status_table_url"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    let url_ok = url.contains("hcch.net") && url.contains("status-table") && url.contains("cid=41");
    let mut model: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut invalid = 0;
    for party in answer["parties"].as_array().into_iter().flatten() {
        let code = party["iso2"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();
        if code.len() != 2 || !code.bytes().all(|b| b.is_ascii_uppercase()) {
            invalid += 1;
            continue;
        }
        let date = party["entry_into_force"]
            .as_str()
            .filter(|d| is_ymd(d))
            .map(str::to_string);
        let name = party["name"].as_str().unwrap_or("").to_string();
        model.insert(code, (name, date));
    }
    let table: Vec<&CountryRow> = rows
        .iter()
        .filter(|r| r.status != "nonparty" && r.iso2.is_some())
        .collect();
    let table_codes: HashSet<&str> = table.iter().filter_map(|r| r.iso2.as_deref()).collect();
    let (mut missing, mut mismatch, mut unsure) = (Vec::new(), Vec::new(), 0);
    for row in &table {
        let code = row.iso2.as_deref().unwrap_or_default();
        match model.get(code) {
            None => missing.push(country_json(row)),
            Some((_, None)) => unsure += 1,
            Some((_, Some(date))) if Some(date) != row.eif.as_ref() => mismatch.push(json!({
                "iso2": code, "name": row.name, "table": row.eif, "model": date,
            })),
            Some(_) => {}
        }
    }
    let mut extra: Vec<Value> = model
        .iter()
        .filter(|(code, _)| !table_codes.contains(code.as_str()))
        .map(|(code, (name, _))| json!({"iso2": code, "name": name}))
        .collect();
    extra.sort_by(|a, b| a["iso2"].as_str().cmp(&b["iso2"].as_str()));
    json!({
        "knowledge_cutoff": answer["knowledge_cutoff"],
        "status_table_url": answer["status_table_url"],
        "status_table_url_ok": url_ok,
        "model_parties": model.len(),
        "table_parties": table.len(),
        "invalid_entries": invalid,
        "missing_in_model": missing,
        "extra_in_model": extra,
        "eif_mismatch": mismatch,
        "eif_unsure": unsure,
    })
}

fn is_ymd(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// Stage 3: authorities and URLs
// ---------------------------------------------------------------------------

/// One authority to ask about.
#[derive(Debug, Clone)]
struct Target {
    slug: String,
    name: String,
    country: String,
    iso2: String,
    /// Hosts the HCCH pages list for this authority (pack `domains`).
    listed_hosts: HashSet<String>,
}

/// Trim list punctuation the HCCH pages leave on names
/// (`"Clerk of the High Court,"`, `"… thereof, and"`).
fn clean_name(name: &str) -> String {
    let mut out = name.trim();
    loop {
        let before = out;
        out = out.trim_end_matches([',', ';']).trim_end();
        for tail in [" and", " or"] {
            if let Some(stripped) = out.strip_suffix(tail) {
                out = stripped.trim_end();
            }
        }
        if out == before {
            return out.to_string();
        }
    }
}

/// Why a world-pack "name" is a page fragment rather than an authority
/// (address, contact or heading line, a description), or `None`.
pub(crate) fn not_a_name(name: &str) -> Option<&'static str> {
    let Some(first) = name.chars().next() else {
        return Some("empty");
    };
    let folded = fold(name);
    if name.chars().count() <= 6 {
        return Some("too short to be a name");
    }
    if first.is_lowercase() {
        return Some("starts in lower case (a description, not a name)");
    }
    if first.is_ascii_digit() {
        return Some("starts with a digit (an address line)");
    }
    if name.ends_with(':') {
        return Some("a heading");
    }
    if name.contains('@')
        || ["telephone", "fax", "tel.", "e-mail", "email protected"]
            .iter()
            .any(|w| folded.contains(w))
    {
        return Some("a contact line");
    }
    let mut words = name.split_whitespace();
    let postcode = words.next().is_some_and(|w| {
        w.split_once('-').is_some_and(|(cc, num)| {
            (1..=3).contains(&cc.len())
                && cc.bytes().all(|b| b.is_ascii_uppercase())
                && num.bytes().all(|b| b.is_ascii_digit())
                && !num.is_empty()
        })
    });
    if postcode
        || name
            .split_whitespace()
            .last()
            .is_some_and(|w| w.bytes().all(|b| b.is_ascii_digit()))
    {
        return Some("an address line");
    }
    None
}

/// Read `packs/world/<ISO2>.json` for the requested countries. Returns
/// the targets plus the skipped fragment names.
fn targets(repo: &Path, scope: &[String]) -> Result<(Vec<Target>, Vec<Value>), JobError> {
    let dir = repo.join("packs/world");
    let mut codes: Vec<String> = if scope.iter().any(|c| c == "ALL") {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| JobError::fatal(format!("{}: {e}", dir.display())))?;
        let mut all: Vec<String> = entries
            .filter_map(|e| e.ok()?.path().file_stem()?.to_str().map(str::to_string))
            .filter(|stem| stem.len() == 2)
            .collect();
        all.sort();
        all
    } else {
        scope.to_vec()
    };
    codes.sort();
    codes.dedup();
    let missing: Vec<&String> = codes
        .iter()
        .filter(|c| !dir.join(format!("{c}.json")).is_file())
        .collect();
    if !missing.is_empty() {
        return Err(JobError::fatal(format!(
            "no world pack for {missing:?} (non-parties go through research.nonparty)"
        )));
    }
    let (mut out, mut skipped) = (Vec::new(), Vec::new());
    for code in codes {
        let path = dir.join(format!("{code}.json"));
        let pack: Pack = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
            .map_err(|e| JobError::fatal(format!("{}: {e}", path.display())))?;
        for authority in pack.authorities {
            let name = clean_name(&authority.name);
            if let Some(reason) = not_a_name(&name) {
                skipped.push(
                    json!({"slug": authority.slug, "name": authority.name, "reason": reason}),
                );
                continue;
            }
            let listed_hosts = pack
                .domains
                .iter()
                .filter(|d| d.authority == authority.slug && d.name != "www.hcch.net")
                .map(|d| d.name.to_ascii_lowercase())
                .collect();
            out.push(Target {
                slug: authority.slug,
                name,
                country: authority.country,
                iso2: code.clone(),
                listed_hosts,
            });
        }
    }
    Ok((out, skipped))
}

/// Government-style host, or one the HCCH pages list for the authority.
fn official_hint(host: &str, target: &Target) -> Option<&'static str> {
    let listed = target.listed_hosts.iter().any(|h| {
        let bare = h.trim_start_matches("www.");
        host == h || host == bare || host.ends_with(&format!(".{bare}"))
    });
    if listed {
        return Some("listed_by_hcch");
    }
    host.split('.')
        .any(|label| GOV_LABELS.contains(&label))
        .then_some("government_domain")
}

/// Significant words of a name (5+ letters), folded.
fn name_words(name: &str) -> Vec<String> {
    fold(name)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 5)
        .map(str::to_string)
        .collect()
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

struct Run<'a, P: ModelPort, F: Fetch> {
    port: &'a P,
    fetch: &'a F,
    model: ModelSpec,
    ledger: Ledger,
    dir: PathBuf,
    run_id: String,
    now: i64,
}

impl<P: ModelPort, F: Fetch> Run<'_, P, F> {
    /// One Spark call, dumped under `calls/<label>-<task>.json` with a
    /// ledger line. `Err` carries the model's error text.
    fn ask(
        &self,
        task: &(TaskSpec, Option<JsonSchema>),
        label: &str,
        user_text: String,
    ) -> Result<Value, String> {
        let started = Instant::now();
        let req = ChatRequest {
            model: self.model.id.clone(),
            instructions: task.0.instructions.clone(),
            user_text,
            schema: task.1.clone(),
            max_output_tokens: self.model.max_output_tokens,
            temperature: 0.1,
            reasoning_effort: None,
            cache_key: Some(format!("atlas-discovery-{}", task.0.name)),
        };
        let reply = self.port.complete_chat(&req);
        let calls = self.dir.join("calls");
        let _ = std::fs::create_dir_all(&calls);
        let dump = calls.join(format!("{label}-{}.json", task.0.name));
        let (text, tokens) = match &reply {
            Ok(c) => (
                c.text.clone(),
                (c.input_tokens, c.output_tokens, c.cached_tokens),
            ),
            Err(e) => (format!("ERROR: {e}"), (0, 0, 0)),
        };
        let _ = std::fs::write(
            &dump,
            serde_json::to_vec_pretty(&json!({
                "model": req.model, "task": task.0.name, "input": req.user_text, "output": text,
            }))
            .expect("dump serializes"),
        );
        let _ = self.ledger.append(&CallRecord {
            run_id: self.run_id.clone(),
            at: self.now,
            task: task.0.name.clone(),
            model: self.model.id.clone(),
            mode: "standard".to_string(),
            effort: "low".to_string(),
            protocol: "fast".to_string(),
            kind: "chat".to_string(),
            input_sha: handbook_store::store::sha256_hex(req.user_text.as_bytes()),
            input_tokens: tokens.0,
            output_tokens: tokens.1,
            cached_tokens: tokens.2,
            cost_usd: atlas::ledger::estimate_cost(&self.model, tokens.0, tokens.1),
            elapsed_ms: started.elapsed().as_millis() as u64,
            output: dump.display().to_string(),
        });
        let completion = reply.map_err(|e| e.to_string())?;
        model_json(&completion.text).ok_or_else(|| "model output is not JSON".to_string())
    }

    /// Fetch and save a page; the body text, or why not.
    fn page_text(&self, url: &str) -> Result<String, String> {
        if host_of(url).is_none() || !url.starts_with("https://") {
            return Err("not an absolute https URL".to_string());
        }
        let bytes = self
            .fetch
            .get(url)
            .map_err(|e| format!("fetch failed: {e}"))?;
        let pages = self.dir.join("pages");
        if std::fs::create_dir_all(&pages).is_ok() {
            let sha = handbook_store::store::sha256_hex(&bytes);
            let _ = std::fs::write(pages.join(format!("{sha}.html")), &bytes);
        }
        Ok(page_text(&String::from_utf8_lossy(&bytes)))
    }

    /// Keep a proposed URL when it answers and names the authority or an
    /// apostille term; otherwise say why it was dropped.
    fn check_url(&self, url: &str, kind: &str, language: &str, target: &Target) -> Value {
        let base = json!({"url": url, "kind": kind, "language": language});
        let text = match self.page_text(url) {
            Ok(text) => fold(&text),
            Err(reason) => return json!({"kept": false, "candidate": base, "reason": reason}),
        };
        let words = name_words(&target.name);
        let hits = words.iter().filter(|w| text.contains(w.as_str())).count();
        let evidence = if !words.is_empty() && hits >= words.len().min(2) {
            "names the authority".to_string()
        } else if let Some(term) = APOSTILLE_TERMS.iter().find(|t| text.contains(*t)) {
            format!("apostille term \"{term}\"")
        } else {
            return json!({"kept": false, "candidate": base,
                          "reason": "page names neither the authority nor an apostille term"});
        };
        let host = host_of(url).unwrap_or_default();
        json!({"kept": true, "candidate": base, "evidence": evidence,
               "official_hint": official_hint(&host, target)})
    }

    /// One `find-urls` batch: ask, map answers back by slug, check URLs.
    fn batch(
        &self,
        task: &(TaskSpec, Option<JsonSchema>),
        index: usize,
        batch: &[Target],
    ) -> (Vec<Value>, bool) {
        let user_text = batch
            .iter()
            .map(|t| format!("{} | {} | {} ({})", t.slug, t.name, t.country, t.iso2))
            .collect::<Vec<_>>()
            .join("\n");
        let answer = self.ask(task, &format!("batch-{index:03}"), user_text);
        let mut by_slug: HashMap<String, &Value> = HashMap::new();
        if let Ok(answer) = &answer {
            for item in answer["items"].as_array().into_iter().flatten() {
                if let Some(slug) = item["slug"].as_str() {
                    by_slug.entry(slug.to_string()).or_insert(item);
                }
            }
        }
        let model_error = answer.as_ref().err().cloned();
        let out = batch
            .iter()
            .map(|t| {
                let head = json!({"slug": t.slug, "name": t.name, "iso2": t.iso2});
                let Some(item) = by_slug.get(&t.slug) else {
                    let why = model_error
                        .clone()
                        .unwrap_or_else(|| "no answer for this slug".into());
                    return json!({"authority": head, "error": why});
                };
                let checked: Vec<Value> = item["urls"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(URLS_PER_AUTHORITY)
                    .filter_map(|u| {
                        let url = u["url"].as_str()?.trim();
                        Some(self.check_url(
                            url,
                            u["kind"].as_str().unwrap_or("other"),
                            u["language"].as_str().unwrap_or(""),
                            t,
                        ))
                    })
                    .collect();
                json!({"authority": head, "unknown": item["unknown"].as_bool().unwrap_or(false),
                       "urls": checked})
            })
            .collect();
        (out, model_error.is_some())
    }
}

/// Load one task (and its schema) from the pack.
fn task_spec(cfg: &AtlasConfig, name: &str) -> Result<(TaskSpec, Option<JsonSchema>), JobError> {
    let spec = cfg
        .tasks
        .iter()
        .find(|t| t.name == name)
        .cloned()
        .ok_or_else(|| JobError::fatal(format!("atlas pack has no task {name}")))?;
    let schema = match spec.schema.as_deref() {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| JobError::fatal(format!("schema {path}: {e}")))?;
            Some(JsonSchema {
                name: name.replace('-', "_"),
                schema: serde_json::from_str(&text)
                    .map_err(|e| JobError::fatal(format!("schema {path}: {e}")))?,
                strict: false,
            })
        }
        None => None,
    };
    Ok((spec, schema))
}

/// `payload.countries`: `"all"` or a list of ISO2 codes (upper-cased).
fn scope(payload: &Value) -> Result<Option<Vec<String>>, JobError> {
    match &payload["countries"] {
        Value::Null => Ok(None),
        Value::String(s) if s.eq_ignore_ascii_case("all") => Ok(Some(vec!["ALL".to_string()])),
        Value::Array(list) => {
            let codes: Vec<String> = list
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_ascii_uppercase)
                .collect();
            if codes.len() != list.len() || codes.is_empty() {
                return Err(JobError::fatal(
                    "countries: a non-empty list of ISO2 codes, or \"all\"",
                ));
            }
            Ok(Some(codes))
        }
        _ => Err(JobError::fatal(
            "countries: a list of ISO2 codes, or \"all\"",
        )),
    }
}

/// Run the whole job against any model port and fetcher (the handler
/// passes the live ones; tests pass stubs).
pub(crate) fn bootstrap<P: ModelPort, F: Fetch>(
    port: &P,
    fetch: &F,
    repo: &Path,
    dir: &Path,
    payload: &Value,
    now: i64,
) -> Result<JobOutput, JobError> {
    let scope = scope(payload)?;
    // Bad country codes fail before any model call is spent.
    let (targets, skipped) = match &scope {
        Some(scope) => targets(repo, scope)?,
        None => (Vec::new(), Vec::new()),
    };
    let batch_size = payload["batch_size"]
        .as_u64()
        .map_or(DEFAULT_BATCH, |n| n as usize)
        .clamp(5, 25);
    let max_batches = payload["max_batches"]
        .as_u64()
        .map_or(DEFAULT_MAX_BATCHES, |n| n as usize)
        .max(1);
    let (cfg, pack_dir) = AtlasConfig::load(&repo.join(PACK_PATH))
        .map_err(|e| JobError::fatal(format!("atlas pack: {e}")))?;
    let source = |name: &str| -> Result<String, JobError> {
        let spec = cfg
            .sources
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| JobError::fatal(format!("atlas pack has no source {name}")))?;
        std::fs::read_to_string(pack_dir.join(&spec.url))
            .map_err(|e| JobError::fatal(format!("source {name} ({}): {e}", spec.url)))
    };
    let crosscheck_task = task_spec(&cfg, "hague-crosscheck")?;
    let urls_task = task_spec(&cfg, "find-urls")?;
    let model = cfg
        .model(&crosscheck_task.0.model)
        .cloned()
        .ok_or_else(|| JobError::fatal("atlas pack: unknown model"))?;
    let run = Run {
        port,
        fetch,
        model,
        ledger: Ledger::new(dir.join("ledger.jsonl")),
        dir: dir.to_path_buf(),
        run_id: format!("discovery-{now}"),
        now,
    };
    std::fs::create_dir_all(dir).map_err(|e| JobError::retry(e.to_string()))?;

    // Stage 1: countries.
    let recorded = payload["recorded"].as_bool().unwrap_or(false);
    let hcch_html = if recorded {
        source("hcch-status")?
    } else {
        let url = crate::world::STATUS_TABLE_URL;
        let bytes = fetch
            .get(url)
            .map_err(|e| JobError::retry(format!("HCCH status table: {e}")))?;
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let today = callcache::utc_date(now);
    let rows = countries(&source("un-members")?, &hcch_html, &today).map_err(JobError::fatal)?;
    let count = |status: &str| rows.iter().filter(|r| r.status == status).count();
    let future: Vec<Value> = rows
        .iter()
        .filter(|r| r.status == "future_party")
        .map(country_json)
        .collect();
    let unmapped: Vec<&str> = rows
        .iter()
        .filter(|r| r.iso2.is_none())
        .map(|r| r.name.as_str())
        .collect();

    // Stage 2: cross-check (a diagnostic: a failure never fails the job).
    let crosscheck = if payload["crosscheck"].as_bool().unwrap_or(true) {
        match run.ask(&crosscheck_task, "all", format!("Today is {today}.")) {
            Ok(answer) => crosscheck_diff(&answer, &rows),
            Err(e) => json!({"error": e}),
        }
    } else {
        json!({"skipped": true})
    };

    // Stage 3: official URLs.
    let (mut authorities, mut not_asked, mut failed_batches, mut batches) = (Vec::new(), 0, 0, 0);
    for (index, chunk) in targets.chunks(batch_size).enumerate() {
        if index >= max_batches {
            not_asked += chunk.len();
            continue;
        }
        eprintln!(
            "discovery.bootstrap: batch {index} ({} authorities)",
            chunk.len()
        );
        let (results, failed) = run.batch(&urls_task, index, chunk);
        batches += 1;
        if failed {
            failed_batches += 1;
        }
        authorities.extend(results);
    }
    if batches > 0 && failed_batches == batches {
        return Err(JobError::retry(format!(
            "every find-urls batch failed at the model ({batches}); see {}",
            dir.join("calls").display()
        )));
    }
    let urls: Vec<&Value> = authorities
        .iter()
        .flat_map(|a| a["urls"].as_array().into_iter().flatten())
        .collect();
    let kept = urls.iter().filter(|u| u["kept"] == true).count();
    let hinted = urls
        .iter()
        .filter(|u| u["kept"] == true && !u["official_hint"].is_null())
        .count();
    let count_of = |key: &str| crosscheck[key].as_array().map(Vec::len);
    let summary = json!({
        "countries": rows.len(), "parties": count("party"),
        "future_parties": count("future_party"), "nonparties": count("nonparty"),
        "unmapped_parties": unmapped.len(),
        "crosscheck_missing": count_of("missing_in_model"),
        "crosscheck_extra": count_of("extra_in_model"),
        "crosscheck_eif_mismatch": count_of("eif_mismatch"),
        "authorities_asked": authorities.len(), "authorities_not_asked": not_asked,
        "names_skipped": skipped.len(), "failed_batches": failed_batches,
        "urls_kept": kept, "urls_official_hint": hinted, "urls_dropped": urls.len() - kept,
    });
    let bundle = json!({
        "format": "discovery.v1", "kind": "discovery.bootstrap", "run": run.run_id,
        "today": today,
        "hcch": {"url": crate::world::STATUS_TABLE_URL, "recorded": recorded,
                 "sha256": handbook_store::store::sha256_hex(hcch_html.as_bytes())},
        "countries": rows.iter().map(country_json).collect::<Vec<_>>(),
        "future_parties": future,
        "unmapped_parties": unmapped,
        "crosscheck": crosscheck,
        "authorities": authorities,
        "names_skipped": skipped,
        "summary": summary,
    });
    Ok(JobOutput::json(&bundle, summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas::model::StubPort;

    /// Canned pages; anything else answers like a 404.
    struct StubFetch(HashMap<String, Vec<u8>>);

    impl Fetch for StubFetch {
        fn get(&self, url: &str) -> Result<Vec<u8>, String> {
            self.0
                .get(url)
                .cloned()
                .ok_or_else(|| "fetch HTTP status: 404".to_string())
        }
    }

    fn fixture(path: &str) -> String {
        std::fs::read_to_string(super::super::repo_root().join(path)).unwrap()
    }

    fn universe() -> Vec<CountryRow> {
        countries(
            &fixture("evals/world-nonparty/un-member-states.html"),
            &fixture("evals/world/status-table-cid41.html"),
            "2026-11-04",
        )
        .unwrap()
    }

    /// 2026-11-04, the day this was written.
    const NOW: i64 = 1_793_750_400;

    #[test]
    fn countries_split_parties_future_parties_and_nonparties() {
        let rows = universe();
        let status = |code: &str| {
            rows.iter()
                .find(|r| r.iso2.as_deref() == Some(code))
                .unwrap_or_else(|| panic!("{code} missing"))
                .clone()
        };
        let th = status("TH");
        assert_eq!(th.status, "future_party");
        assert_eq!(th.eif.as_deref(), Some("2027-02-28"));
        assert_eq!(status("VN").status, "party");
        assert_eq!(status("DZ").status, "party");
        assert_eq!(status("XK").status, "party");
        assert_eq!(status("EG").status, "nonparty");
        assert_eq!(status("VA").status, "nonparty");
        // 130 table rows (127 UN members + Cook Islands, Niue, Kosovo),
        // 66 UN members outside the table, two observers.
        assert_eq!(rows.iter().filter(|r| r.status != "nonparty").count(), 130);
        assert_eq!(rows.len(), 130 + (193 - 127) + 2);
        let codes: HashSet<&str> = rows.iter().filter_map(|r| r.iso2.as_deref()).collect();
        assert_eq!(codes.len(), rows.len(), "one row per country");
        // A day after Thailand's EIF it is a party like the rest.
        let later = countries(
            &fixture("evals/world-nonparty/un-member-states.html"),
            &fixture("evals/world/status-table-cid41.html"),
            "2027-02-28",
        )
        .unwrap();
        assert!(later.iter().all(|r| r.status != "future_party"));
    }

    #[test]
    fn a_broken_status_table_fails_instead_of_shrinking_the_world() {
        let err = countries(
            &fixture("evals/world-nonparty/un-member-states.html"),
            "<table><tr><td>Albania</td></tr></table>",
            "2026-11-04",
        )
        .unwrap_err();
        assert!(err.contains("changed shape"), "{err}");
    }

    #[test]
    fn crosscheck_reports_missing_extra_and_wrong_dates() {
        let rows = universe();
        let mut parties: Vec<Value> = rows
            .iter()
            .filter(|r| r.status != "nonparty")
            .filter(|r| !matches!(r.iso2.as_deref(), Some("TH" | "DZ")))
            .map(|r| json!({"iso2": r.iso2, "name": r.name, "entry_into_force": r.eif}))
            .collect();
        for p in &mut parties {
            if p["iso2"] == "VN" {
                p["entry_into_force"] = json!("2026-10-01");
            }
            if p["iso2"] == "FR" {
                p["entry_into_force"] = Value::Null;
            }
        }
        parties.push(json!({"iso2": "EG", "name": "Egypt", "entry_into_force": "2025-01-01"}));
        parties.push(json!({"iso2": "not-a-code", "name": "?", "entry_into_force": null}));
        let answer = json!({
            "status_table_url": "https://www.hcch.net/en/instruments/conventions/status-table/?cid=41",
            "knowledge_cutoff": "2025-06",
            "parties": parties,
        });
        let diff = crosscheck_diff(&answer, &rows);
        assert_eq!(diff["status_table_url_ok"], true);
        let codes = |key: &str| -> Vec<String> {
            diff[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["iso2"].as_str().unwrap().to_string())
                .collect()
        };
        let mut missing = codes("missing_in_model");
        missing.sort();
        assert_eq!(missing, ["DZ", "TH"]);
        assert_eq!(codes("extra_in_model"), ["EG"]);
        assert_eq!(codes("eif_mismatch"), ["VN"]);
        assert_eq!(diff["eif_mismatch"][0]["table"], "2026-09-11");
        assert_eq!(diff["eif_unsure"], 1);
        assert_eq!(diff["invalid_entries"], 1);
        let wrong_url = crosscheck_diff(&json!({"status_table_url": "https://hcch.net/"}), &rows);
        assert_eq!(wrong_url["status_table_url_ok"], false);
        assert_eq!(wrong_url["missing_in_model"].as_array().unwrap().len(), 130);
    }

    #[test]
    fn fragment_names_are_skipped_and_list_punctuation_trimmed() {
        assert_eq!(
            clean_name("Maritime Administrator and Special Agents thereof, and"),
            "Maritime Administrator and Special Agents thereof"
        );
        assert_eq!(
            clean_name("Clerk and Deputy Clerk of the High Court,"),
            "Clerk and Deputy Clerk of the High Court"
        );
        for fragment in [
            "Limmatquai 50",
            "CH-8001 Zurich",
            "Telephone: +41-44-268-2211",
            "Fax: +41-44-268-2212",
            "[email protected]",
            "Declaration of 2 August 2004:",
            "a notary,",
            "Land",
        ] {
            assert!(not_a_name(&clean_name(fragment)).is_some(), "{fragment}");
        }
        for name in [
            "Minister of Foreign Affairs of the Marshall Islands",
            "Oberlandesgericht",
            "Department of Consular Affairs of the Ministry of Foreign Affairs",
            "IRI Corporate and Maritime Services (Switzerland) A.G.",
        ] {
            assert_eq!(not_a_name(name), None, "{name}");
        }
    }

    #[test]
    fn official_hints() {
        let target = Target {
            slug: "x".into(),
            name: "x".into(),
            country: "x".into(),
            iso2: "DE".into(),
            listed_hosts: HashSet::from(["www.bfaa.diplo.example".to_string()]),
        };
        assert_eq!(
            official_hint("bfaa.diplo.example", &target),
            Some("listed_by_hcch")
        );
        assert_eq!(
            official_hint("www.mofa.go.jp", &target),
            Some("government_domain")
        );
        assert_eq!(
            official_hint("travel.state.gov", &target),
            Some("government_domain")
        );
        assert_eq!(official_hint("apostille-now.example", &target), None);
    }

    /// End to end on stubs, recorded status table, Marshall Islands world
    /// pack: fragments skipped, two find-urls batches, URLs checked.
    #[test]
    fn bootstrap_reports_countries_crosscheck_and_checked_urls() {
        const MFA: &str = "https://mfa.gov.mh.example/apostille";
        const REGISTRY: &str = "https://registry.example/corporations";
        const OFFTOPIC: &str = "https://news.example/story";
        let fetch = StubFetch(HashMap::from([
            (
                MFA.to_string(),
                b"<h1>Apostille</h1><p>Requests go to the Ministry.</p>".to_vec(),
            ),
            (
                REGISTRY.to_string(),
                b"<p>Office of the Deputy Registrar of corporations</p>".to_vec(),
            ),
            (OFFTOPIC.to_string(), b"<p>Weather today.</p>".to_vec()),
        ]));
        let crosscheck = json!({
            "status_table_url": "https://www.hcch.net/en/instruments/conventions/status-table/?cid=41",
            "knowledge_cutoff": "2025-06",
            "parties": [{"iso2": "US", "name": "United States", "entry_into_force": "1981-10-15"}],
        });
        let batch1 = json!({"items": [
            {"slug": "mh-ca-1", "unknown": false, "urls": [
                {"url": MFA, "kind": "apostille_page", "language": "en"},
                {"url": "https://invented.example/x", "kind": "other"},
                {"url": "http://plain.example/", "kind": "other"}
            ]},
            {"slug": "mh-ca-2", "unknown": true, "urls": []},
            {"slug": "not-in-batch", "unknown": false, "urls": [{"url": OFFTOPIC, "kind": "other"}]}
        ]});
        let batch2 = json!({"items": [
            {"slug": "mh-ca-10", "unknown": false, "urls": [
                {"url": REGISTRY, "kind": "authority_home"},
                {"url": OFFTOPIC, "kind": "other"}
            ]}
        ]});
        let port = StubPort::with_script(vec![
            crosscheck.to_string(),
            batch1.to_string(),
            batch2.to_string(),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let out = bootstrap(
            &port,
            &fetch,
            &super::super::repo_root(),
            dir.path(),
            &json!({"recorded": true, "countries": ["mh"], "batch_size": 5}),
            NOW,
        )
        .unwrap();
        let s = &out.summary;
        assert_eq!(s["future_parties"], 1, "{s}");
        assert_eq!(s["parties"], 129);
        assert_eq!(s["crosscheck_missing"], 129, "the stub knew only the US");
        // 16 Marshall Islands rows: 7 fragments, 9 names in two batches.
        assert_eq!(s["names_skipped"], 7);
        assert_eq!(s["authorities_asked"], 9);
        assert_eq!(s["failed_batches"], 0);
        assert_eq!(s["urls_kept"], 2);
        assert_eq!(s["urls_official_hint"], 1);
        assert_eq!(s["urls_dropped"], 3);
        let bundle: Value = serde_json::from_slice(&out.bytes).unwrap();
        assert_eq!(bundle["format"], "discovery.v1");
        assert_eq!(bundle["future_parties"][0]["iso2"], "TH");
        let by_slug = |slug: &str| {
            bundle["authorities"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["authority"]["slug"] == slug)
                .unwrap_or_else(|| panic!("{slug} missing"))
                .clone()
        };
        let mfa = by_slug("mh-ca-1");
        assert_eq!(mfa["urls"][0]["kept"], true);
        assert_eq!(mfa["urls"][0]["official_hint"], "government_domain");
        assert_eq!(
            mfa["urls"][1]["reason"],
            "fetch failed: fetch HTTP status: 404"
        );
        assert_eq!(mfa["urls"][2]["reason"], "not an absolute https URL");
        assert_eq!(by_slug("mh-ca-2")["unknown"], true);
        assert_eq!(by_slug("mh-ca-3")["error"], "no answer for this slug");
        let registry = by_slug("mh-ca-10");
        assert_eq!(registry["urls"][0]["evidence"], "names the authority");
        assert!(registry["urls"][0]["official_hint"].is_null());
        assert_eq!(
            registry["urls"][1]["reason"],
            "page names neither the authority nor an apostille term"
        );
        // Every call is dumped and ledgered.
        assert!(dir.path().join("calls/all-hague-crosscheck.json").is_file());
        assert!(dir.path().join("calls/batch-001-find-urls.json").is_file());
        assert!(dir.path().join("ledger.jsonl").is_file());
    }

    #[test]
    fn payload_errors_are_fatal() {
        let port = StubPort::with_script(vec![]);
        let fetch = StubFetch(HashMap::new());
        let dir = tempfile::tempdir().unwrap();
        let repo = super::super::repo_root();
        for (payload, want) in [
            (
                json!({"recorded": true, "countries": ["EG"]}),
                "no world pack",
            ),
            (json!({"recorded": true, "countries": "DE"}), "countries"),
            (json!({"recorded": true, "countries": []}), "countries"),
        ] {
            let err = bootstrap(&port, &fetch, &repo, dir.path(), &payload, NOW).unwrap_err();
            assert!(!err.retryable, "{payload}");
            assert!(err.message.contains(want), "{payload}: {}", err.message);
        }
        // The live table is fetched unless `recorded`; a failed fetch retries.
        let err = bootstrap(&port, &fetch, &repo, dir.path(), &json!({}), NOW).unwrap_err();
        assert!(err.retryable, "{}", err.message);
    }
}
