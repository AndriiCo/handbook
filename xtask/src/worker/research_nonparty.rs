//! `research.nonparty` (P392): legalization packs for UN member states
//! that are not Apostille Convention parties, plus the Holy See and the
//! State of Palestine.
//!
//! The atlas pack `crates/atlas/packs/nonparty-sources.toml` owns the
//! model, prompts and schemas; this file owns the orchestration:
//!
//! 1. Countries = UN members (`evals/world-nonparty/un-member-states.html`)
//!    minus HCCH 1961 parties (`evals/world/status-table-cid41.html`),
//!    plus the two observer states. Deterministic code, no model.
//! 2. Per country, `find-sources` (Spark) proposes the US DOS Judicial
//!    Assistance page and MFA/embassy legalization pages.
//! 3. Every candidate (plus the conventional DOS URL and the DOS page's
//!    own embassy/MFA links) is fetched with `ingest::fetch`. A URL
//!    survives only when it answers 2xx and its text names the country;
//!    the DOS page must also sit on `travel.state.gov`.
//! 4. `extract-chain` (Spark) reads the surviving pages and returns the
//!    legalizing authority and chain steps, each with a verbatim quote.
//! 5. Quote check: a value stays only when its quote (≥ 12 chars) occurs
//!    in the fetched text of the page it cites (entity-decoded,
//!    tag-stripped, lowercased, quote/dash/diacritic-folded,
//!    whitespace-collapsed). Everything dropped is listed in the report.
//! 6. One pack per country, shaped like `packs/egypt-consular.json`
//!    (`kind: country` authority, unverified domains, weekly watch
//!    seeds, `needs_review` chain + observations), validated with
//!    `packs::validate`, slugs checked against `packs/*.json` and
//!    `packs/world/*.json`.
//!
//! Every fetched page lands in `<job>/pages/`, every model call's input
//! and output in `<job>/calls/` plus an atlas ledger, and the packs in
//! `<job>/packs/world-nonparty/<ISO2>.json`. The uploaded result is the
//! `packs.v1` bundle that `cargo xtask worker --unpack` writes out.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use atlas::config::{AtlasConfig, ModelSpec, TaskSpec};
use atlas::ledger::{CallRecord, Ledger};
use atlas::model::{ChatRequest, JsonSchema, ModelPort};
use serde_json::{Value, json};

use super::{JobContext, JobError, JobHandler, JobOutput};
use crate::packs::{Pack, PackAuthority, PackDomain, PackObservation, PackStep, PackWatch};

/// The atlas pack (repo-relative).
pub const PACK_PATH: &str = "crates/atlas/packs/nonparty-sources.toml";
/// Where the packs belong in the repo (`--unpack` default).
pub const OUT_DIR: &str = "packs/world-nonparty";
/// Validation backlog ref for every step and observation.
const VALIDATION_REF: &str = "W-8";
const DOS_HOST: &str = "travel.state.gov";
const DOS_BASE: &str =
    "https://travel.state.gov/content/travel/en/legal/Judicial-Assistance-Country-Information/";
/// Shorter quotes prove nothing about provenance.
const MIN_QUOTE_CHARS: usize = 12;
const MAX_QUOTE_CHARS: usize = 400;
/// Non-DOS candidate pages fetched per country.
const MAX_CANDIDATES: usize = 8;
/// Pages (DOS first) handed to `extract-chain`.
const MAX_EXTRACT_PAGES: usize = 4;
/// Characters of each page's text sent to the model.
const PAGE_TEXT_CAP: usize = 24_000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// One request per two seconds per host (review §13 politeness).
const POLITENESS: Duration = Duration::from_secs(2);
/// Link text or URL words that mark a DOS outbound link worth a fetch.
const LINK_HINTS: [&str; 8] = [
    "embassy",
    "consul",
    "legaliz",
    "legalis",
    "foreign",
    "mfa",
    "ministry",
    "authenticat",
];

/// Observer states listed beside the 193 members (operator scope).
const OBSERVERS: [(&str, &str); 2] = [("Holy See", "VA"), ("State of Palestine", "PS")];

pub struct ResearchNonparty;

impl JobHandler for ResearchNonparty {
    fn kind(&self) -> &'static str {
        "research.nonparty"
    }

    fn run(&self, ctx: &JobContext, payload: &Value) -> Result<JobOutput, JobError> {
        let Some(key) = ctx.env.get("META_API_KEY") else {
            return Err(JobError::fatal(
                "research.nonparty needs META_API_KEY in worker.env",
            ));
        };
        let base = ctx
            .env
            .get("META_BASE_URL")
            .unwrap_or_else(|| atlas::model::BASE_URL.to_string());
        // Every Meta call and fetch goes through the call cache (P391):
        // a rerun replays saved answers and pages instead of paying
        // again; only live calls count toward the daily cap.
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
        research(
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
// Countries
// ---------------------------------------------------------------------------

/// One country to research.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Country {
    pub name: String,
    pub iso2: &'static str,
}

/// Member names off the UN member-states page: the text of every
/// `<h2 …>` inside the member list (one per member).
pub(crate) fn un_members(html: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<h2") {
        let after = &rest[start..];
        let Some(open_end) = after.find('>') else {
            break;
        };
        let body = &after[open_end + 1..];
        let Some(close) = body.find("</h2>") else {
            break;
        };
        let name = clean_text(&body[..close]);
        if un_iso2(&name).is_some() {
            names.push(name);
        }
        rest = &body[close..];
    }
    names
}

/// UN member-state name (as un.org spells it) to ISO 3166-1 alpha-2.
fn un_iso2(name: &str) -> Option<&'static str> {
    Some(match fold(name).as_str() {
        "afghanistan" => "AF",
        "albania" => "AL",
        "algeria" => "DZ",
        "andorra" => "AD",
        "angola" => "AO",
        "antigua and barbuda" => "AG",
        "argentina" => "AR",
        "armenia" => "AM",
        "australia" => "AU",
        "austria" => "AT",
        "azerbaijan" => "AZ",
        "bahamas" | "bahamas (the)" => "BS",
        "bahrain" => "BH",
        "bangladesh" => "BD",
        "barbados" => "BB",
        "belarus" => "BY",
        "belgium" => "BE",
        "belize" => "BZ",
        "benin" => "BJ",
        "bhutan" => "BT",
        "bolivia (plurinational state of)" => "BO",
        "bosnia and herzegovina" => "BA",
        "botswana" => "BW",
        "brazil" => "BR",
        "brunei darussalam" => "BN",
        "bulgaria" => "BG",
        "burkina faso" => "BF",
        "burundi" => "BI",
        "cabo verde" => "CV",
        "cambodia" => "KH",
        "cameroon" => "CM",
        "canada" => "CA",
        "central african republic" => "CF",
        "chad" => "TD",
        "chile" => "CL",
        "china" | "china (the people's republic of)" => "CN",
        "colombia" => "CO",
        "comoros" => "KM",
        "congo" => "CG",
        "costa rica" => "CR",
        "cote d'ivoire" => "CI",
        "croatia" => "HR",
        "cuba" => "CU",
        "cyprus" => "CY",
        "czechia" => "CZ",
        "democratic people's republic of korea" => "KP",
        "democratic republic of the congo" => "CD",
        "denmark" => "DK",
        "djibouti" => "DJ",
        "dominica" => "DM",
        "dominican republic" => "DO",
        "ecuador" => "EC",
        "egypt" => "EG",
        "el salvador" => "SV",
        "equatorial guinea" => "GQ",
        "eritrea" => "ER",
        "estonia" => "EE",
        "eswatini" => "SZ",
        "ethiopia" => "ET",
        "fiji" => "FJ",
        "finland" => "FI",
        "france" => "FR",
        "gabon" => "GA",
        "gambia" | "gambia (republic of the)" => "GM",
        "georgia" => "GE",
        "germany" => "DE",
        "ghana" => "GH",
        "greece" => "GR",
        "grenada" => "GD",
        "guatemala" => "GT",
        "guinea" => "GN",
        "guinea bissau" | "guinea-bissau" => "GW",
        "guyana" => "GY",
        "haiti" => "HT",
        "honduras" => "HN",
        "hungary" => "HU",
        "iceland" => "IS",
        "india" => "IN",
        "indonesia" => "ID",
        "iran (islamic republic of)" => "IR",
        "iraq" => "IQ",
        "ireland" => "IE",
        "israel" => "IL",
        "italy" => "IT",
        "jamaica" => "JM",
        "japan" => "JP",
        "jordan" => "JO",
        "kazakhstan" => "KZ",
        "kenya" => "KE",
        "kiribati" => "KI",
        "kuwait" => "KW",
        "kyrgyzstan" => "KG",
        "lao people's democratic republic" => "LA",
        "latvia" => "LV",
        "lebanon" => "LB",
        "lesotho" => "LS",
        "liberia" => "LR",
        "libya" => "LY",
        "liechtenstein" => "LI",
        "lithuania" => "LT",
        "luxembourg" => "LU",
        "madagascar" => "MG",
        "malawi" => "MW",
        "malaysia" => "MY",
        "maldives" => "MV",
        "mali" => "ML",
        "malta" => "MT",
        "marshall islands" => "MH",
        "mauritania" => "MR",
        "mauritius" => "MU",
        "mexico" => "MX",
        "micronesia (federated states of)" => "FM",
        "monaco" => "MC",
        "mongolia" => "MN",
        "montenegro" => "ME",
        "morocco" => "MA",
        "mozambique" => "MZ",
        "myanmar" => "MM",
        "namibia" => "NA",
        // un.org lists Nauru under its own-language name.
        "nauru" | "naoero" => "NR",
        "nepal" => "NP",
        "netherlands (kingdom of the)" | "netherlands" => "NL",
        "new zealand" => "NZ",
        "nicaragua" => "NI",
        "niger" => "NE",
        "nigeria" => "NG",
        "north macedonia" => "MK",
        "norway" => "NO",
        "oman" => "OM",
        "pakistan" => "PK",
        "palau" => "PW",
        "panama" => "PA",
        "papua new guinea" => "PG",
        "paraguay" => "PY",
        "peru" => "PE",
        "philippines" => "PH",
        "poland" => "PL",
        "portugal" => "PT",
        "qatar" => "QA",
        "republic of korea" => "KR",
        "republic of moldova" => "MD",
        "romania" => "RO",
        "russian federation" => "RU",
        "rwanda" => "RW",
        "saint kitts and nevis" => "KN",
        "saint lucia" => "LC",
        "saint vincent and the grenadines" => "VC",
        "samoa" => "WS",
        "san marino" => "SM",
        "sao tome and principe" => "ST",
        "saudi arabia" => "SA",
        "senegal" => "SN",
        "serbia" => "RS",
        "seychelles" => "SC",
        "sierra leone" => "SL",
        "singapore" => "SG",
        "slovakia" => "SK",
        "slovenia" => "SI",
        "solomon islands" => "SB",
        "somalia" => "SO",
        "south africa" => "ZA",
        "south sudan" => "SS",
        "spain" => "ES",
        "sri lanka" => "LK",
        "sudan" => "SD",
        "suriname" => "SR",
        "sweden" => "SE",
        "switzerland" => "CH",
        "syrian arab republic" => "SY",
        "tajikistan" => "TJ",
        "thailand" => "TH",
        "timor-leste" => "TL",
        "togo" => "TG",
        "tonga" => "TO",
        "trinidad and tobago" => "TT",
        "tunisia" => "TN",
        "turkiye" => "TR",
        "turkmenistan" => "TM",
        "tuvalu" => "TV",
        "uganda" => "UG",
        "ukraine" => "UA",
        "united arab emirates" => "AE",
        "united kingdom of great britain and northern ireland" => "GB",
        "united republic of tanzania" => "TZ",
        "united states of america" => "US",
        "uruguay" => "UY",
        "uzbekistan" => "UZ",
        "vanuatu" => "VU",
        "venezuela (bolivarian republic of)" | "venezuela, bolivarian republic of" => "VE",
        "viet nam" => "VN",
        "yemen" => "YE",
        "zambia" => "ZM",
        "zimbabwe" => "ZW",
        _ => return None,
    })
}

/// UN members minus HCCH parties (matched by ISO code, so spelling
/// differences between the two tables cannot leak a party in), plus
/// the observer states. Errors when a fixture row has no code.
pub(crate) fn nonparty_countries(un_html: &str, hcch_html: &str) -> Result<Vec<Country>, String> {
    let members = un_members(un_html);
    if members.len() != 193 {
        return Err(format!(
            "UN fixture lists {} members, want 193",
            members.len()
        ));
    }
    let mut party_codes = HashSet::new();
    for party in crate::world::party_names(hcch_html) {
        let Some(code) = crate::world::party_iso2(&party) else {
            return Err(format!("HCCH party {party:?} has no ISO code"));
        };
        party_codes.insert(code);
    }
    let mut out: Vec<Country> = members
        .into_iter()
        .filter_map(|name| {
            let iso2 = un_iso2(&name)?;
            let name = if iso2 == "NR" {
                "Nauru".to_string()
            } else {
                name
            };
            (!party_codes.contains(iso2)).then_some(Country { name, iso2 })
        })
        .collect();
    for (name, iso2) in OBSERVERS {
        if !party_codes.contains(iso2) {
            out.push(Country {
                name: name.to_string(),
                iso2,
            });
        }
    }
    Ok(out)
}

/// Names a page may use for the country (UN long form, short head,
/// common English forms), folded.
fn name_variants(name: &str) -> Vec<String> {
    let mut out = vec![fold(name)];
    for sep in [" (", ","] {
        if let Some((head, _)) = name.split_once(sep) {
            out.push(fold(head));
        }
    }
    let extra: &[&str] = match fold(name).as_str() {
        "democratic people's republic of korea" => &["north korea", "dprk"],
        "lao people's democratic republic" => &["laos"],
        "syrian arab republic" => &["syria"],
        "united republic of tanzania" => &["tanzania"],
        "cote d'ivoire" => &["ivory coast"],
        "democratic republic of the congo" => &["congo"],
        "state of palestine" => &["palestin"],
        "holy see" => &["vatican"],
        "timor-leste" => &["east timor", "timor"],
        "guinea bissau" => &["guinea-bissau"],
        _ => &[],
    };
    out.extend(extra.iter().map(|s| s.to_string()));
    out.retain(|v| v.chars().count() >= 4);
    out.sort();
    out.dedup();
    out
}

/// `…/Judicial-Assistance-Country-Information/<Short-Name>.html`.
fn conventional_dos_url(name: &str) -> String {
    let head = name.split_once(" (").map_or(name, |(h, _)| h);
    let slug: String = fold(head)
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |c| {
                c.to_uppercase().chain(chars).collect::<String>()
            })
        })
        .collect::<Vec<_>>()
        .join("-")
        .replace('\'', "");
    format!("{DOS_BASE}{slug}.html")
}

// ---------------------------------------------------------------------------
// Text normal form (quote check)
// ---------------------------------------------------------------------------

/// Lowercase, fold quotes/dashes/common Latin diacritics, collapse
/// whitespace. Both sides of every comparison go through this.
pub(crate) fn fold(text: &str) -> String {
    let mapped: String = text
        .chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            '’' | '‘' | '`' | '´' | 'ʼ' => '\'',
            '“' | '”' | '«' | '»' => '"',
            '–' | '—' | '‑' | '‐' => '-',
            '\u{a0}' => ' ',
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Visible page text: script/style/noscript/svg blocks dropped, tags
/// to spaces, entities decoded, whitespace collapsed.
pub(crate) fn page_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let after = &rest[lt..];
        let end = after.find('>').map_or(after.len(), |i| i + 1);
        let tag = after[..end].to_ascii_lowercase();
        let skip = ["script", "style", "noscript", "svg"]
            .into_iter()
            .find(|name| tag.starts_with(&format!("<{name}")));
        rest = match skip {
            Some(name) => {
                let closer = format!("</{name}");
                let tail = &after[end..];
                match tail.to_ascii_lowercase().find(&closer) {
                    Some(at) => {
                        let tail = &tail[at..];
                        tail.find('>').map_or("", |i| &tail[i + 1..])
                    }
                    None => "",
                }
            }
            None => &after[end..],
        };
        out.push(' ');
    }
    out.push_str(rest);
    crate::world::collapse_ws(&crate::world::html_unescape(&out))
}

fn clean_text(fragment: &str) -> String {
    page_text(fragment)
}

fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    (!host.is_empty() && host.contains('.') && !host.contains('@') && !host.contains(':'))
        .then(|| host.to_lowercase())
}

/// Absolute http(s) links on a page with their anchor text.
fn links(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("<a ") {
        let after = &rest[at..];
        let Some(open_end) = after.find('>') else {
            break;
        };
        let open = &after[..open_end];
        let body = &after[open_end + 1..];
        let close = body.find("</a>").unwrap_or(body.len());
        if let Some(href) = attr(open, "href")
            && host_of(&href).is_some()
        {
            out.push((href, clean_text(&body[..close])));
        }
        rest = &body[close.min(body.len())..];
    }
    out
}

fn attr(tag: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let marker = format!("{name}={quote}");
        if let Some(start) = tag.find(&marker) {
            let value = &tag[start + marker.len()..];
            let end = value.find(quote)?;
            return Some(crate::world::html_unescape(&value[..end]));
        }
    }
    None
}

/// Cut on a char boundary at or below `max` bytes.
fn clip(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

// ---------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------

/// The one network seam (tests stub it).
pub(crate) trait Fetch {
    /// Body of a 2xx answer, or why not.
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// `ingest::fetch` with per-host politeness and a URL-keyed disk cache
/// (`<cache-dir>/fetch-cache`), so reruns re-read instead of re-fetch.
pub(crate) struct LiveFetch {
    cache: callcache::FetchCache,
    last: RefCell<HashMap<String, Instant>>,
}

impl LiveFetch {
    pub(crate) fn new(cache: callcache::FetchCache) -> Self {
        Self {
            cache,
            last: RefCell::new(HashMap::new()),
        }
    }
}

impl Fetch for LiveFetch {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let host = host_of(url).ok_or_else(|| "not an absolute http(s) URL".to_string())?;
        // Politeness only before a live request: a saved page replays
        // from the fetch cache without touching the host.
        let saved = matches!(self.cache.lookup(url), Ok(Some(_)));
        if !saved || self.cache.mode() == callcache::CacheMode::Refresh {
            if let Some(at) = self.last.borrow().get(&host)
                && at.elapsed() < POLITENESS
            {
                std::thread::sleep(POLITENESS - at.elapsed());
            }
            self.last.borrow_mut().insert(host, Instant::now());
        }
        ingest::fetch::fetch_url_cached(&self.cache, url, FETCH_TIMEOUT)
            .map(|fetched| fetched.bytes)
            .map_err(|e| e.to_string())
    }
}

/// One fetched page that names the country.
struct Page {
    url: String,
    /// `dos | mfa | embassy | consulate | other | link`.
    kind: String,
    html: String,
    text: String,
    folded: String,
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

struct Run<'a, P: ModelPort, F: Fetch> {
    port: &'a P,
    fetch: &'a F,
    model: ModelSpec,
    find: (TaskSpec, Option<JsonSchema>),
    extract: (TaskSpec, Option<JsonSchema>),
    ledger: Ledger,
    dir: PathBuf,
    run_id: String,
    now: i64,
    /// Slugs already used by `packs/*.json` and `packs/world/*.json`.
    taken: HashSet<String>,
}

/// Per-country outcome for the report.
struct Outcome {
    pack: Option<Pack>,
    report: Value,
    model_error: bool,
}

/// Run the whole job against any model port and fetcher (the handler
/// passes the live ones; tests pass stubs). Writes packs under
/// `<dir>/packs/world-nonparty/` and returns the bundle.
pub(crate) fn research<P: ModelPort, F: Fetch>(
    port: &P,
    fetch: &F,
    repo: &Path,
    dir: &Path,
    payload: &Value,
    now: i64,
) -> Result<JobOutput, JobError> {
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
    let mut countries = nonparty_countries(&source("un-members")?, &source("hcch-status")?)
        .map_err(JobError::fatal)?;
    countries = select(countries, payload)?;
    let task = |name: &str| -> Result<(TaskSpec, Option<JsonSchema>), JobError> {
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
    };
    let find = task("find-sources")?;
    let extract = task("extract-chain")?;
    let model = cfg
        .model(&find.0.model)
        .cloned()
        .ok_or_else(|| JobError::fatal("atlas pack: unknown model"))?;
    let run = Run {
        port,
        fetch,
        model,
        find,
        extract,
        ledger: Ledger::new(dir.join("ledger.jsonl")),
        dir: dir.to_path_buf(),
        run_id: format!("nonparty-{now}"),
        now,
        taken: existing_slugs(repo),
    };
    let out_dir = dir.join(OUT_DIR);
    std::fs::create_dir_all(&out_dir).map_err(|e| JobError::retry(e.to_string()))?;
    let (mut packs, mut report, mut model_errors) = (Vec::new(), Vec::new(), 0);
    for country in &countries {
        eprintln!("research.nonparty: {} {}", country.iso2, country.name);
        let outcome = run.country(country)?;
        if outcome.model_error {
            model_errors += 1;
        }
        if let Some(pack) = outcome.pack {
            let bytes = pack_bytes(&pack);
            std::fs::write(out_dir.join(format!("{}.json", pack.pack)), &bytes)
                .map_err(|e| JobError::retry(e.to_string()))?;
            packs.push(pack);
        }
        report.push(outcome.report);
    }
    if !countries.is_empty() && model_errors == countries.len() {
        return Err(JobError::retry(format!(
            "every country failed at the model ({model_errors}); see {}",
            dir.join("calls").display()
        )));
    }
    let kept: u64 = report
        .iter()
        .map(|r| r["steps_kept"].as_u64().unwrap_or(0))
        .sum();
    let dropped: u64 = report
        .iter()
        .map(|r| r["steps_dropped"].as_u64().unwrap_or(0))
        .sum();
    let summary = json!({
        "countries": countries.len(), "packs": packs.len(),
        "steps_kept": kept, "steps_dropped": dropped, "model_errors": model_errors,
    });
    let bundle = json!({
        "format": "packs.v1", "kind": "research.nonparty", "run": run.run_id,
        "packs": packs, "report": report,
    });
    Ok(JobOutput::json(&bundle, summary))
}

/// Apply `payload.countries` (ISO2 list) and `payload.limit`.
fn select(countries: Vec<Country>, payload: &Value) -> Result<Vec<Country>, JobError> {
    let mut out = match payload.get("countries").and_then(Value::as_array) {
        Some(wanted) => {
            let wanted: Vec<String> = wanted
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_ascii_uppercase)
                .collect();
            let unknown: Vec<&String> = wanted
                .iter()
                .filter(|code| !countries.iter().any(|c| c.iso2 == code.as_str()))
                .collect();
            if !unknown.is_empty() {
                return Err(JobError::fatal(format!(
                    "not a non-party country code: {unknown:?} (parties live in packs/world)"
                )));
            }
            countries
                .into_iter()
                .filter(|c| wanted.iter().any(|w| w == c.iso2))
                .collect()
        }
        None => countries,
    };
    if let Some(limit) = payload.get("limit").and_then(Value::as_u64) {
        out.truncate(limit as usize);
    }
    Ok(out)
}

/// Authority slugs in `packs/*.json` and `packs/world/*.json`.
fn existing_slugs(repo: &Path) -> HashSet<String> {
    let mut slugs = HashSet::new();
    for dir in [repo.join("packs"), repo.join("packs/world")] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let Ok(pack) = std::fs::read(&path).map(|b| serde_json::from_slice::<Pack>(&b)) else {
                continue;
            };
            if let Ok(pack) = pack {
                slugs.extend(pack.authorities.into_iter().map(|a| a.slug));
            }
        }
    }
    slugs
}

fn pack_bytes(pack: &Pack) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(pack).expect("pack serializes");
    bytes.push(b'\n');
    bytes
}

/// Parse model output as JSON (whole text, else the outermost braces).
fn model_json(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok().or_else(|| {
        let (start, end) = (text.find('{')?, text.rfind('}')?);
        (start < end).then(|| serde_json::from_str(&text[start..=end]).ok())?
    })
}

impl<P: ModelPort, F: Fetch> Run<'_, P, F> {
    /// One Spark call, with its input/output dumped under `calls/` and
    /// a ledger line. `Err` carries the model's error text.
    fn ask(
        &self,
        task: &(TaskSpec, Option<JsonSchema>),
        iso2: &str,
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
            cache_key: Some(format!("atlas-nonparty-{}", task.0.name)),
        };
        let reply = self.port.complete_chat(&req);
        let calls = self.dir.join("calls");
        let _ = std::fs::create_dir_all(&calls);
        let dump = calls.join(format!("{iso2}-{}.json", task.0.name));
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

    /// Fetch `url`; keep it when it answers and names the country.
    fn page(
        &self,
        url: &str,
        kind: &str,
        variants: &[String],
        dropped: &mut Vec<Value>,
    ) -> Option<Page> {
        let mut drop = |reason: String| {
            dropped.push(json!({"what": "url", "url": url, "kind": kind, "reason": reason}));
            None
        };
        if host_of(url).is_none() || !url.starts_with("https://") {
            return drop("not an absolute https URL".to_string());
        }
        let bytes = match self.fetch.get(url) {
            Ok(bytes) => bytes,
            Err(e) => return drop(format!("fetch failed: {e}")),
        };
        let sha = handbook_store::store::sha256_hex(&bytes);
        let pages = self.dir.join("pages");
        if std::fs::create_dir_all(&pages).is_ok() {
            let _ = std::fs::write(pages.join(format!("{sha}.html")), &bytes);
        }
        let html = String::from_utf8_lossy(&bytes).into_owned();
        let text = page_text(&html);
        let folded = fold(&text);
        if !variants.iter().any(|v| folded.contains(v.as_str())) {
            return drop("page does not name the country".to_string());
        }
        Some(Page {
            url: url.to_string(),
            kind: kind.to_string(),
            html,
            text,
            folded,
        })
    }

    fn country(&self, country: &Country) -> Result<Outcome, JobError> {
        let iso2 = country.iso2;
        let variants = name_variants(&country.name);
        let mut dropped = Vec::new();
        let mut report = json!({"iso2": iso2, "country": country.name});
        let fatal_model = |e: &str| e.starts_with("auth") || e.starts_with("billing");

        // 1. Spark proposes sources.
        let user = format!("Country: {}\nISO 3166-1 alpha-2: {iso2}", country.name);
        let found = match self.ask(&self.find, iso2, user) {
            Ok(found) => found,
            Err(e) if fatal_model(&e) => return Err(JobError::fatal(format!("meta api: {e}"))),
            Err(e) => {
                report["error"] = json!(format!("find-sources: {e}"));
                return Ok(Outcome {
                    pack: None,
                    report,
                    model_error: true,
                });
            }
        };

        // 2. The DOS page: Spark's pick, then the conventional URL.
        let mut dos_candidates: Vec<String> = Vec::new();
        if let Some(url) = found["dos_url"].as_str() {
            dos_candidates.push(url.trim().to_string());
        }
        dos_candidates.push(conventional_dos_url(&country.name));
        dos_candidates.dedup();
        let mut dos: Option<Page> = None;
        for url in &dos_candidates {
            if host_of(url).as_deref() != Some(DOS_HOST) {
                dropped.push(json!({"what": "url", "url": url, "kind": "dos",
                                    "reason": "DOS page must be on travel.state.gov"}));
                continue;
            }
            if let Some(page) = self.page(url, "dos", &variants, &mut dropped) {
                dos = Some(page);
                break;
            }
        }

        // 3. MFA / embassy pages: Spark's list, then the DOS page's links.
        let mut candidates: Vec<(String, String)> = found["pages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                let url = p["url"].as_str()?.trim().to_string();
                let kind = p["kind"].as_str().unwrap_or("other").to_string();
                Some((url, kind))
            })
            .collect();
        for (href, text) in dos.as_ref().map_or_else(Vec::new, |p| links(&p.html)) {
            let hay = format!("{} {}", href.to_lowercase(), fold(&text));
            if host_of(&href).as_deref() != Some(DOS_HOST)
                && LINK_HINTS.iter().any(|hint| hay.contains(hint))
            {
                candidates.push((href, "link".to_string()));
            }
        }
        let mut seen: HashSet<String> = dos.iter().map(|p| p.url.clone()).collect();
        candidates.retain(|(url, _)| seen.insert(url.clone()));
        let mut others = Vec::new();
        for (url, kind) in candidates.iter().take(MAX_CANDIDATES) {
            if host_of(url).as_deref() == Some(DOS_HOST) {
                continue;
            }
            if let Some(page) = self.page(url, kind, &variants, &mut dropped) {
                others.push(page);
            }
        }
        let pages: Vec<&Page> = dos.iter().chain(others.iter()).collect();
        report["dos_url"] = json!(dos.as_ref().map(|p| &p.url));
        report["pages"] = json!(pages.iter().map(|p| &p.url).collect::<Vec<_>>());

        // 4. Spark extracts the chain from the surviving pages.
        let mut steps: Vec<(String, String, String, String)> = Vec::new();
        let mut authority: Option<(String, String, String)> = None;
        let mut steps_dropped = 0u64;
        let mut model_error = false;
        if !pages.is_empty() {
            let mut user = format!("Country: {} ({iso2})\n", country.name);
            for (n, page) in pages.iter().take(MAX_EXTRACT_PAGES).enumerate() {
                user.push_str(&format!(
                    "\nPAGE {} {}\n{}\n",
                    n + 1,
                    page.url,
                    clip(&page.text, PAGE_TEXT_CAP)
                ));
            }
            match self.ask(&self.extract, iso2, user) {
                Ok(chain) => {
                    let check = |v: &Value, what: &str, dropped: &mut Vec<Value>| {
                        quote_checked(v, &pages, what, dropped)
                    };
                    if let Some((url, quote)) =
                        check(&chain["authority"], "authority", &mut dropped)
                    {
                        let name = chain["authority"]["name"].as_str().unwrap_or("").trim();
                        if !name.is_empty() {
                            authority = Some((clip(name, 200).to_string(), url, quote));
                        }
                    }
                    for step in chain["steps"].as_array().into_iter().flatten() {
                        match check(step, "step", &mut dropped) {
                            Some((url, quote)) => steps.push((
                                clip(step["title"].as_str().unwrap_or("Step").trim(), 120)
                                    .to_string(),
                                clip(step["detail"].as_str().unwrap_or("").trim(), 400).to_string(),
                                url,
                                quote,
                            )),
                            None => steps_dropped += 1,
                        }
                    }
                }
                Err(e) if fatal_model(&e) => {
                    return Err(JobError::fatal(format!("meta api: {e}")));
                }
                Err(e) => {
                    model_error = true;
                    report["error"] = json!(format!("extract-chain: {e}"));
                }
            }
        }
        report["steps_kept"] = json!(steps.len());
        report["steps_dropped"] = json!(steps_dropped);
        report["authority"] = json!(authority.as_ref().map(|a| &a.0));

        // 5. The pack.
        let pack = self.build_pack(country, &pages, &steps, authority.as_ref());
        let bytes = pack_bytes(&pack);
        let violations = crate::packs::validate(&pack, iso2, &bytes);
        report["dropped"] = json!(dropped);
        if !violations.is_empty() {
            report["error"] = json!(format!("pack rejected: {}", violations.join("; ")));
            return Ok(Outcome {
                pack: None,
                report,
                model_error,
            });
        }
        report["slug"] = json!(pack.authorities[0].slug);
        Ok(Outcome {
            pack: Some(pack),
            report,
            model_error,
        })
    }

    fn build_pack(
        &self,
        country: &Country,
        pages: &[&Page],
        steps: &[(String, String, String, String)],
        authority: Option<&(String, String, String)>,
    ) -> Pack {
        let iso2 = country.iso2;
        let base = iso2.to_ascii_lowercase();
        let slug = [format!("{base}-legalization"), format!("{base}-nonparty")]
            .into_iter()
            .find(|s| !self.taken.contains(s))
            .unwrap_or_else(|| format!("{base}-nonparty-{}", self.now));
        let mut domains: Vec<PackDomain> = Vec::new();
        let mut watch = Vec::new();
        for page in pages.iter().take(5) {
            let Some(host) = host_of(&page.url) else {
                continue;
            };
            if !domains.iter().any(|d| d.name == host) {
                domains.push(PackDomain {
                    name: host,
                    authority: slug.clone(),
                    status: "unverified".to_string(),
                    notes: format!(
                        "Fetched by research.nonparty ({}); names {}. Staff capture pending.",
                        self.run_id, country.name
                    ),
                });
            }
            let notes = if page.kind == "dos" {
                "US DOS Judicial Assistance country page (authentication of documents).".to_string()
            } else {
                format!(
                    "{} page; fetched and checked to name {}.",
                    page.kind, country.name
                )
            };
            watch.push(PackWatch {
                authority: slug.clone(),
                kind: "website_official".to_string(),
                uri: page.url.clone(),
                cadence: "weekly".to_string(),
                notes,
            });
        }
        let mut chain_steps = Vec::new();
        let mut observations = Vec::new();
        for (n, (title, detail, url, quote)) in steps.iter().enumerate() {
            let host = host_of(url).unwrap_or_default();
            let detail = if detail.is_empty() {
                format!("Quoted from {host}: \u{201c}{quote}\u{201d}")
            } else {
                format!("{detail} Quoted from {host}: \u{201c}{quote}\u{201d}")
            };
            chain_steps.push(PackStep {
                id: format!("{base}-np-{}", n + 1),
                authority: slug.clone(),
                order: n as u32,
                title: title.clone(),
                detail,
                status: "needs_review".to_string(),
                validation: VALIDATION_REF.to_string(),
            });
            observations.push(PackObservation {
                authority: slug.clone(),
                topic: "legalization_chain".to_string(),
                statement: format!("OBSERVED ({host}): \u{201c}{quote}\u{201d}"),
                source_uri: url.clone(),
                status: "needs_review".to_string(),
                validation: VALIDATION_REF.to_string(),
            });
        }
        if chain_steps.is_empty() {
            chain_steps.push(PackStep {
                id: format!("{base}-np-1"),
                authority: slug.clone(),
                order: 0,
                title: "Consular legalization (not an Apostille party): chain unresearched"
                    .to_string(),
                detail: format!(
                    "No chain step survived the quote check in {}; read the watch pages.",
                    self.run_id
                ),
                status: "needs_review".to_string(),
                validation: VALIDATION_REF.to_string(),
            });
        }
        if let Some((name, url, quote)) = authority {
            let host = host_of(url).unwrap_or_default();
            observations.push(PackObservation {
                authority: slug.clone(),
                topic: "authentication".to_string(),
                statement: format!(
                    "OBSERVED ({host}): final legalization by {name}: \u{201c}{quote}\u{201d}"
                ),
                source_uri: url.clone(),
                status: "needs_review".to_string(),
                validation: VALIDATION_REF.to_string(),
            });
        }
        Pack {
            pack: iso2.to_string(),
            version: 1,
            title: format!(
                "{} — consular legalization, not an Apostille party \
                 (research.nonparty, needs review)",
                country.name
            ),
            authorities: vec![PackAuthority {
                kind: "country".to_string(),
                country: country.name.clone(),
                region: String::new(),
                name: format!("{} — consular legalization", country.name),
                slug,
            }],
            domains,
            watch,
            chain_steps,
            observations,
        }
    }
}

/// `(source_url, quote)` when `value`'s quote occurs on the page it
/// cites; otherwise records why it was dropped.
fn quote_checked(
    value: &Value,
    pages: &[&Page],
    what: &str,
    dropped: &mut Vec<Value>,
) -> Option<(String, String)> {
    if value.is_null() {
        return None;
    }
    let url = value["source_url"].as_str().unwrap_or("").trim();
    let quote = value["quote"].as_str().unwrap_or("").trim();
    let mut drop = |reason: &str| {
        dropped
            .push(json!({"what": what, "url": url, "quote": clip(quote, 120), "reason": reason}));
        None
    };
    let Some(page) = pages.iter().find(|p| p.url == url) else {
        return drop("source_url is not a fetched page");
    };
    let n = quote.chars().count();
    if !(MIN_QUOTE_CHARS..=MAX_QUOTE_CHARS).contains(&n) {
        return drop("quote length out of range");
    }
    if !page.folded.contains(&fold(quote)) {
        return drop("quote not found on the cited page");
    }
    Some((url.to_string(), quote.to_string()))
}

/// `--unpack <bundle> --into <dir>`: write and validate each pack.
pub fn unpack(bundle: &Path, into: &Path) -> Result<usize, String> {
    let bytes = std::fs::read(bundle).map_err(|e| format!("{}: {e}", bundle.display()))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| format!("bundle: {e}"))?;
    if value["format"] != "packs.v1" {
        return Err("not a packs.v1 bundle".to_string());
    }
    let packs: Vec<Pack> =
        serde_json::from_value(value["packs"].clone()).map_err(|e| format!("bundle packs: {e}"))?;
    let mut errors = Vec::new();
    for pack in &packs {
        let bytes = pack_bytes(pack);
        let violations = crate::packs::validate(pack, &pack.pack, &bytes);
        if !violations.is_empty() {
            errors.push(format!("{}: {}", pack.pack, violations.join("; ")));
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    std::fs::create_dir_all(into).map_err(|e| format!("{}: {e}", into.display()))?;
    for pack in &packs {
        let path = into.join(format!("{}.json", pack.pack));
        std::fs::write(&path, pack_bytes(pack)).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(packs.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas::model::StubPort;

    const DOS: &str = concat!(
        "https://travel.state.gov/content/travel/en/legal/",
        "Judicial-Assistance-Country-Information/Egypt.html"
    );
    const EMBASSY: &str = "https://consulate.egypt.example/legalization";

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

    fn fixtures() -> (String, String) {
        let repo = super::super::repo_root();
        (
            std::fs::read_to_string(repo.join("evals/world-nonparty/un-member-states.html"))
                .unwrap(),
            std::fs::read_to_string(repo.join("evals/world/status-table-cid41.html")).unwrap(),
        )
    }

    #[test]
    fn country_list_is_un_members_minus_parties_plus_observers() {
        let (un, hcch) = fixtures();
        assert_eq!(un_members(&un).len(), 193);
        let list = nonparty_countries(&un, &hcch).unwrap();
        let codes: Vec<&str> = list.iter().map(|c| c.iso2).collect();
        for want in ["EG", "AF", "CU", "VA", "PS"] {
            assert!(codes.contains(&want), "{want} missing from {codes:?}");
        }
        for party in ["US", "FR", "CN", "VN", "NL"] {
            assert!(!codes.contains(&party), "{party} is an Apostille party");
        }
        let unique: HashSet<&str> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len());
        // 193 members, 127 of the 130 parties are members (Cook Islands,
        // Niue and Kosovo are not), plus two observers.
        let parties = crate::world::party_names(&hcch).len();
        assert_eq!(list.len(), 193 - (parties - 3) + 2, "{codes:?}");
        // No world pack exists for a non-party (the world index is parties).
        let repo = super::super::repo_root();
        for c in &list {
            assert!(
                !repo.join(format!("packs/world/{}.json", c.iso2)).exists(),
                "{}",
                c.iso2
            );
        }
    }

    #[test]
    fn text_folding_and_page_text() {
        assert_eq!(fold("Côte D’Ivoire\u{a0} —  x"), "cote d'ivoire - x");
        let html =
            r#"<p>A&amp;B <b>c</b></p><script>var x = "<p>no</p>";</script><style>p{}</style>z"#;
        assert_eq!(page_text(html), "A&B c z");
        assert_eq!(
            conventional_dos_url("Iran (Islamic Republic of)"),
            format!("{DOS_BASE}Iran.html")
        );
        assert_eq!(
            conventional_dos_url("Democratic People's Republic of Korea"),
            format!("{DOS_BASE}Democratic-Peoples-Republic-Of-Korea.html")
        );
        assert!(name_variants("Iran (Islamic Republic of)").contains(&"iran".to_string()));
        let found =
            links(r#"<a class="x" href="https://e.example/a?b=1&amp;c=2">Embassy <i>site</i></a>"#);
        assert_eq!(
            found,
            [(
                "https://e.example/a?b=1&c=2".to_string(),
                "Embassy site".to_string()
            )]
        );
    }

    fn egypt_run(dir: &Path) -> JobOutput {
        let dos_html = format!(
            "<html><head><script>var t = 'Egypt';</script></head><body><h1>Egypt</h1>\
             <p>Authentication of Documents: Documents issued in the United States must be \
             authenticated by the U.S. Department of State before the Embassy of Egypt \
             legalizes them.</p><a href=\"{EMBASSY}\">Embassy of Egypt legalization</a>\
             <a href=\"https://travel.state.gov/other.html\">Other</a></body></html>"
        );
        let embassy_html = "<p>Consulate of Egypt, consular section.</p><p>The consulate \
                            accepts only documents authenticated by the Secretary of State \
                            and the U.S. Department of State.</p>";
        let fetch = StubFetch(HashMap::from([
            (DOS.to_string(), dos_html.into_bytes()),
            (EMBASSY.to_string(), embassy_html.as_bytes().to_vec()),
            (
                "https://unrelated.example/page".to_string(),
                b"<p>Nothing about the country here.</p>".to_vec(),
            ),
        ]));
        let find = json!({
            "dos_url": DOS,
            "pages": [
                {"url": "https://invented.example/legalisation", "kind": "mfa", "why": "guess"},
                {"url": "https://unrelated.example/page", "kind": "other", "why": "x"}
            ]
        });
        let chain = json!({
            "authority": {"name": "Consulate of Egypt", "source_url": EMBASSY,
                          "quote": "The consulate accepts only documents authenticated"},
            "steps": [
                {"title": "US Department of State authentication",
                 "detail": "Authenticate the document at the Department of State.",
                 "source_url": DOS,
                 "quote": "must be authenticated by the U.S. Department of State"},
                {"title": "Invented fee", "detail": "", "source_url": EMBASSY,
                 "quote": "The fee is 47 dollars per document"},
                {"title": "Wrong page", "source_url": "https://elsewhere.example/",
                 "quote": "must be authenticated by the U.S. Department of State"}
            ],
            "notes": ""
        });
        let port = StubPort::with_script(vec![find.to_string(), chain.to_string()]);
        let repo = super::super::repo_root();
        research(
            &port,
            &fetch,
            &repo,
            dir,
            &json!({"countries": ["eg"]}),
            1_793_750_400,
        )
        .unwrap()
    }

    /// End to end on stubs: Spark proposals, fetch + name check, quote
    /// check, a valid pack on disk and in the bundle, drops reported.
    #[test]
    fn research_writes_a_valid_quote_checked_pack() {
        let dir = tempfile::tempdir().unwrap();
        let out = egypt_run(dir.path());
        assert_eq!(out.content_type, "application/json");
        assert_eq!(out.summary["packs"], 1);
        assert_eq!(out.summary["steps_kept"], 1);
        assert_eq!(out.summary["steps_dropped"], 2);
        let bundle: Value = serde_json::from_slice(&out.bytes).unwrap();
        assert_eq!(bundle["format"], "packs.v1");
        let pack: Pack = serde_json::from_value(bundle["packs"][0].clone()).unwrap();
        assert_eq!(pack.pack, "EG");
        assert_eq!(pack.authorities[0].kind, "country");
        assert_eq!(pack.authorities[0].slug, "eg-legalization");
        assert_eq!(pack.chain_steps.len(), 1);
        assert!(
            pack.chain_steps[0]
                .detail
                .contains("must be authenticated by the U.S. Department of State")
        );
        assert!(pack.chain_steps.iter().all(|s| s.status == "needs_review"));
        let watched: Vec<&str> = pack.watch.iter().map(|w| w.uri.as_str()).collect();
        assert_eq!(
            watched,
            [DOS, EMBASSY],
            "only fetched pages that name Egypt"
        );
        assert_eq!(pack.observations.len(), 2);
        assert!(
            pack.observations
                .iter()
                .any(|o| o.topic == "authentication")
        );
        // On disk, and clean under the repo's pack rules.
        let packs_dir = dir.path().join(OUT_DIR);
        assert!(packs_dir.join("EG.json").is_file());
        assert!(crate::packs::validate_dir(&packs_dir).is_empty());
        // Every drop is explained.
        let report = &bundle["report"][0];
        let reasons: Vec<&str> = report["dropped"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| d["reason"].as_str())
            .collect();
        assert!(
            reasons.iter().any(|r| r.starts_with("fetch failed")),
            "{reasons:?}"
        );
        assert!(
            reasons.contains(&"page does not name the country"),
            "{reasons:?}"
        );
        assert!(
            reasons.contains(&"quote not found on the cited page"),
            "{reasons:?}"
        );
        assert!(
            reasons.contains(&"source_url is not a fetched page"),
            "{reasons:?}"
        );
        // Provenance: both model calls and both pages are on disk.
        assert!(dir.path().join("calls/EG-find-sources.json").is_file());
        assert!(dir.path().join("calls/EG-extract-chain.json").is_file());
        assert_eq!(
            std::fs::read_dir(dir.path().join("pages")).unwrap().count(),
            3
        );
        let ledger = std::fs::read_to_string(dir.path().join("ledger.jsonl")).unwrap();
        assert_eq!(ledger.lines().count(), 2);
        // The bundle unpacks into a directory the validator accepts.
        let bundle_path = dir.path().join("bundle.json");
        std::fs::write(&bundle_path, &out.bytes).unwrap();
        let into = dir.path().join("unpacked");
        assert_eq!(unpack(&bundle_path, &into).unwrap(), 1);
        assert!(crate::packs::validate_dir(&into).is_empty());
    }

    #[test]
    fn nothing_verified_still_yields_an_honest_pack() {
        let dir = tempfile::tempdir().unwrap();
        let fetch = StubFetch(HashMap::new());
        let port = StubPort::with_script(vec![json!({"dos_url": null, "pages": []}).to_string()]);
        let repo = super::super::repo_root();
        let out = research(
            &port,
            &fetch,
            &repo,
            dir.path(),
            &json!({"countries": ["ER"]}),
            1,
        )
        .unwrap();
        let bundle: Value = serde_json::from_slice(&out.bytes).unwrap();
        let pack: Pack = serde_json::from_value(bundle["packs"][0].clone()).unwrap();
        assert!(pack.watch.is_empty() && pack.domains.is_empty());
        assert_eq!(pack.chain_steps.len(), 1);
        assert!(pack.chain_steps[0].title.contains("chain unresearched"));
        assert!(pack.observations.is_empty());
    }

    #[test]
    fn payload_selects_and_rejects_codes() {
        let countries = vec![
            Country {
                name: "Egypt".into(),
                iso2: "EG",
            },
            Country {
                name: "Eritrea".into(),
                iso2: "ER",
            },
        ];
        assert_eq!(
            select(countries.clone(), &json!({"limit": 1}))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            select(countries.clone(), &json!({"countries": ["er"]})).unwrap()[0].iso2,
            "ER"
        );
        let err = select(countries, &json!({"countries": ["US"]})).unwrap_err();
        assert!(err.message.contains("US") && !err.retryable);
    }

    #[test]
    fn slugs_avoid_existing_packs() {
        let taken = existing_slugs(&super::super::repo_root());
        assert!(taken.contains("egypt"), "packs/egypt-consular.json is read");
        assert!(taken.contains("ad-ca-1"), "packs/world is read");
        assert!(!taken.contains("eg-legalization"));
    }
}
