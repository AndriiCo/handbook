//! Project task runner. Use `cargo xtask <task>` instead of shell scripts.
//!
//! Keeping tasks in Rust reuses the project's language, libraries, and
//! error handling instead of scattering logic across shell dialects.

use std::process::Command;

use handbook_store::store::Store;

mod a2_record;
mod accept;
mod admin_staff;
mod archive;
mod ask;
mod atlas;
mod backup;
mod build_handbook;
mod call_cache;
mod config;
mod config_import;
mod crawl;
mod demo_vietnam;
mod eval;
mod fetch_slug;
mod form_scaffold;
mod gold_export;
mod instruct;
mod judge;
mod mcp_serve;
mod packs;
mod pii_sweep;
mod provision;
mod release;
mod secrets;
mod segments;
mod setup;
mod triage;
mod up;
mod voice_replay;
mod watch;
mod worker;
mod world;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("--help" | "-h" | "help") => print_help(),
        Some("ci") => run_ci(),
        Some("init") => run_init(&args[1..]),
        Some("add-authority") => run_add_authority(&args[1..]),
        Some("retention-sweep") => run_retention_sweep(&args[1..]),
        Some("analyze") => run_analyze(&args[1..]),
        Some("fetch-url") => run_fetch_url(&args[1..]),
        Some("fetch-slug") => fetch_slug::run(&args[1..]),
        Some("atlas") => atlas::run(&args[1..]),
        Some("instruct-sweep") => instruct::run_sweep(&args[1..]),
        Some("instruct-demo") => instruct::run_demo(&args[1..]),
        Some("watch-sweep") => watch::run_sweep(&args[1..]),
        Some("hcch-poll") => watch::run_hcch(&args[1..]),
        Some("mail-poll") => watch::run_mail(&args[1..]),
        Some("build-handbook") => build_handbook::run(&args[1..]),
        Some("mcp-serve") => mcp_serve::run(&args[1..]),
        Some("pii-sweep") => pii_sweep::run(&args[1..]),
        Some("form-scaffold") => form_scaffold::run(&args[1..]),
        Some("demo-vietnam") => demo_vietnam::run(&args[1..]),
        Some("eval") => eval::run(&args[1..]),
        Some("gold-export") => gold_export::run(&args[1..]),
        Some("backup") => backup::run(&args[1..]),
        Some("backup-verify") => backup::run_verify(&args[1..]),
        Some("packs-validate") => run_packs_validate(&args[1..]),
        Some("packs-load") => run_packs_load(&args[1..]),
        Some("config-enroll") => config::run_enroll(&args[1..]),
        Some("config-set") => config::run_set(&args[1..]),
        Some("config-import") => config_import::run(&args[1..]),
        Some("config-get") => config::run_get(&args[1..]),
        Some("config-verify") => config::run_verify(&args[1..]),
        Some("config-rotate") => config::run_rotate(&args[1..]),
        Some("setup-key-mint") => config::run_setup_key_mint(&args[1..]),
        Some("admin-invite") => admin_staff::run_invite(&args[1..]),
        Some("admin-offboard") => admin_staff::run_offboard(&args[1..]),
        Some("ask") => ask::run(&args[1..]),
        Some("a2-record") => a2_record::run(&args[1..]),
        Some("archive-package") => archive::run_package(&args[1..]),
        Some("world-index") => world::run(&args[1..]),
        Some("worker") => worker::run(&args[1..]),
        Some("crawl") => crawl::run(&args[1..]),
        Some("segments") => segments::run(&args[1..]),
        Some("triage") => triage::run(&args[1..]),
        Some("secrets-scan") => run_secrets_scan(&args[1..]),
        Some("up") => up::run(&args[1..]),
        Some("judge") => judge::run(&args[1..]),
        Some("model-gate") => run_model_gate(),
        Some("voice-replay") => voice_replay::run(&args[1..]),
        Some("deploy") => release::run_deploy(&args[1..]),
        Some("provision") => provision::run(&args[1..]),
        Some("setup") => setup::run(&args[1..]),
        Some("harden-check") => setup::build::run_harden_check(&args[1..]),
        Some("rollback") => release::run_rollback(&args[1..]),
        Some("install-hooks") => run_install_hooks(),
        Some(unknown) => {
            eprintln!("unknown task: {unknown}\n");
            print_help();
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!(
        "Usage: cargo xtask <task> [flags]

Tasks:
  ci     fmt --check, clippy, and tests (what CI runs)
  init   create a store and apply migrations [--store <path>]
  add-authority   add an office/jurisdiction (see flags below)
  retention-sweep delete voice audio older than 365 days [--store <path>] [--actor <name>]
  analyze     draft proposed facts for an authority (stub default, --live for Meta API, --live --record <corpus>, --replay <corpus>)
  eval        grade drafts on evals/authorities/*.json [--live [--record <corpus>]] [--replay <corpus>] [--local (env LOCAL_MODEL_URL, LOCAL_MODEL, LOCAL_MODEL_KEY?)] [--store <path>] [--evals <dir>] (writes quality.json)
  gold-export real gold cases from a store's accepted facts on official captures [--store <path>] [--out evals/authorities-real] [--per-authority 10]
  fetch-url   fetch one URL into a warc capture (thin watch)
  fetch-slug  sweep one authority's watch URLs (manual, no scheduler)
  atlas  run an Atlas task pack: --config <pack> --task <name> [--effort --model --protocol --out --refresh --limit --live --dry-run --run-id --pdf-tool]; --batch-poll <job>; --check
  instruct-sweep  propose instruction updates from new captures [--store --pack --slug --model --actor --live --dry-run]
  instruct-demo  frozen WA drill on a temp store (no flags)
  watch-sweep poll due watch URLs across authorities (timer runs this)
  hcch-poll   poll the HCCH status table, task on change (timer runs this)
  mail-poll   poll Gmail into email captures (timer runs this)
  build-handbook accepted-only static build + sitemap/robots [--out --base-url]
  mcp-serve   serve lookup_authority on 127.0.0.1 [--port]
  pii-sweep   fail closed on PII in publishable surfaces
  form-scaffold  per-form skeleton: --authority --code --title [--out] [--force]; --check compares
  demo-vietnam  frozen VN drill on a temp store (never prod)
  backup   nightly backup: SQLite backup API + tar (see flags below)
  backup-verify  restore-to-scratch + checks (drill-3 engine)
  packs-validate  validate packs/*.json (F13; needs_review invariants)
  packs-load  load packs into a store [--store <path>] (never writes facts)
  config-enroll|set|get|verify|rotate  encrypted config (see secure-config.md)
  config-import  seal many keys from one JSON payload in one transaction (--file|--stdin, --mode merge|replace, --dry-run, --enroll-if-empty)
  setup-key-mint  mint the first-boot setup key (§9; prints once)
  admin-invite  staff enrolment invite (spec customer-auth.md §5.1)
  admin-offboard  staff offboard: sessions, credentials, lock (§5.6)
  secrets-scan scan tracked files for secrets [--staged for the index]
  crawl   fetch packs/world seeds into WARC/1.1 drops [--packs --out]
  segments  split HCCH authority pages into labelled, quoted segments with a Meta model (local endpoint, or --spark) [--pages <dir>] [--iso2 XX] [--out] (report only)
  triage  sort saved pages with the local Meta model (env LOCAL_MODEL_URL, LOCAL_MODEL) --pages <dir> [--pack <world pack>] [--out] (report only)
  archive-package  bundle a crawl run into WACZ/CSVs/index [--run]
  ask     answer one question with cited authority facts [<question> --slug --base --replay]
  a2-record  record the photo-precheck demo verdicts --png-dir --out-dir [--sam-backend open-weights --mask]
  world-index  build packs/world from HCCH status table [--live] [--out]
  worker  plant worker for the production work queue [--connect <wss>] [--run-local <job.json>] [--unpack <bundle>] [--kinds]
  up    boot the handbook on loopback with synthetic data and a local worker [--check] [--fresh] [--live] [--prebuilt] [--port-base N] [--from-archive <url|dir>]
  judge one-command local copy for hackathon judges: prerequisite doctor, fresh boot, setup key and tour [--live] [--check] [--doctor] [--port-base N]
  model-gate  fail on non-Meta model ids/endpoints in tracked non-doc files
  voice-replay  replay webhook fixtures at the stub (see --corpus/--scenario)
  install-hooks point git at .githooks/ (pre-commit secret scan)
  setup  rebuild targets from private/config.yaml (sops); no file: create it in the localhost wizard. Subcommands: wizard, check, print, new, wipe (setup --help)
  harden-check <elf>...  fail unless PIE, NX stack, no W+X segment, full RELRO, stack protector
  deploy      install a staged release on this host (see flags below; docs/ops/deploy.md)
  rollback    roll back to a kept release (see flags below)
  provision   root, on a server: users, dirs, KEK, wrappers, sudoers, units, firewall; first install (--manifest --release --version --config-stdin --check)

deploy flags:
  --version <sha>  release dir name, 7..=64 lowercase hex (required)
  --src <dir>   staged dir holding web, xtask, spa/, spa-admin/, static/ (+ packs/, evals/) (required)
  --root <path> releases root (default: /srv/apostille)
  --sha <sha>   full sha for the manifest (default: git rev-parse HEAD)
  --packs <dir> packs dir for the packs-validate gate (default: packs)
  --skip-packs  skip the packs-validate gate
  --expect-build <n>  build_no for the accept gate (required unless --skip-accept)
  --skip-accept skip the accept gate
  --store <path>  store for the accept gate (default: store)
  --migration-notes <text>  manifest migration notes (default: empty)
  --rollback-unsafe  mark the release rollback-unsafe in the manifest
  --smoke-url <url>  health URL to smoke (default: https://andrii.ai/apostille/healthz)
  --no-restart  skip systemctl restart (tests only, never prod)
  --allow-today-overlap  accepted for old orders; no day gate exists
  --actor <a>   audit actor (default: xtask)

rollback flags:
  --to <sha>    kept release to restore (required)
  --root <path> releases root (default: /srv/apostille)
  --force --backup-note <text>  override the hard-migration guard
  --smoke-url <url>  health URL to verify (default: prod healthz)
  --no-restart  skip systemctl restart (tests only, never prod)

analyze flags:
  --store <path>  store root (default: store)
  --slug <s>      authority slug, e.g. usa-wa-sos (required)
  --lang <code>   evidence language tag (default: en; EN-only slice)
  --input <text>  evidence text, or @path/to/file (required)
  --actor <a>     audit actor (default: xtask)
  --scan-captures Option-B mode: analyze captures with id > watermark (no --input)
  --limit <n>     max captures per scan (default: 25, scan mode only)
  --live          draft via the Meta Model API (key from sealed meta.api_key)
  --check         probe the Meta API key only (GET /models, no drafting)

fetch-url flags:
  --store <path>  store root (default: store)
  --slug <s>      authority slug, e.g. usa-wa-sos (required)
  --url <u>       URL to fetch (required)
  --source-kind <k> website_official | website_thirdparty (default: website_official)
  --trust <t>     official | staff_verified | community (default: official)
  --actor <a>     audit actor (default: xtask)

fetch-slug flags:
  --store <path>  store root (default: store)
  --slug <s>      authority slug, e.g. usa-wa-sos (required)
  --actor <a>     audit actor (default: xtask)
  --trust <t>     official | staff_verified | community (default: official)
  --limit <n>     max watch URLs per run (default: 25)

watch-sweep flags:
  --store <path>  store root (default: store)
  --actor <a>     audit actor (default: xtask)
  --trust <t>     official | staff_verified | community (default: official)
  --limit <n>     max due URLs per run (default: 25)

hcch-poll flags:
  --store <path>  store root (default: store)
  --actor <a>     audit actor (default: xtask)

mail-poll flags:
  --store <path>  store root (default: store)
  --actor <a>     audit actor (default: xtask)

backup flags:
  --store <path>  store root (default: store)
  --dest <path>   backups root (default: /var/lib/apostille-backups)

backup-verify flags:
  --backup <dir>  daily backup dir (required)
  --scratch <dir> missing-or-empty scratch dir for the restore (required)

add-authority flags:
  --store <path>  store root (default: store)
  --kind <kind>   country | state | province | office (required)
  --country <c>   e.g. USA (required)
  --region <r>    e.g. Washington (default: empty)
  --name <n>      display name (required)
  --slug <s>      unique slug, e.g. usa-wa-sos (required)
  --actor <a>     audit actor (default: xtask)"
    );
}

/// Value of `--name <value>` in task args, if present.
pub(crate) fn flag(args: &[String], name: &str) -> Option<String> {
    args.windows(2).find_map(|pair| {
        if pair[0] == format!("--{name}") {
            Some(pair[1].clone())
        } else {
            None
        }
    })
}

pub(crate) fn required(args: &[String], name: &str) -> String {
    flag(args, name).unwrap_or_else(|| {
        eprintln!("missing required flag: --{name}");
        std::process::exit(2);
    })
}

pub(crate) fn open_store(args: &[String]) -> Store {
    Store::new(flag(args, "store").unwrap_or_else(|| "store".to_string()))
}

/// Database handle for an already-opened store (F6: one open path — tasks
/// must not reimplement the open-and-handle-error sequence themselves).
pub(crate) fn open_db(store: &Store) -> rusqlite::Connection {
    handbook_store::db::open(store).unwrap_or_else(|e| {
        eprintln!("open database (run `cargo xtask init` first): {e}");
        std::process::exit(1);
    })
}

fn run_ci() {
    // Secrets first: fail fast before compiling anything else.
    run_secrets_scan(&[]);
    // Organizer rule next, same speed: no non-Meta model may be
    // called (or configured) at runtime.
    run_model_gate();
    for args in [
        ["fmt", "--all", "--check"].as_slice(),
        [
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ]
        .as_slice(),
        ["test", "--workspace"].as_slice(),
    ] {
        run_or_exit("cargo", args);
    }
    // Web gates (Slice F: these ran manually every phase — now they
    // are part of `ci` so a green `ci` means the whole monorepo).
    // Order matters: unit tests, then types, then the import gate,
    // then both builds (smoke needs them fresh from this run).
    // (`web-budget` is advisory-only since 2026-10-28 — the author
    // removed the size gate; initial-page optimization is deferred.)
    run_or_exit("node", &["--test", "web/test/**/*.test.mjs"]);
    run_or_exit("npm", &["run", "check", "--prefix", "web"]);
    run_or_exit("npm", &["run", "check:imports", "--prefix", "web"]);
    run_or_exit("npm", &["run", "build", "--prefix", "web"]);
    run_or_exit("npm", &["run", "build:admin", "--prefix", "web"]);
    run_or_exit("npm", &["run", "smoke", "--prefix", "web"]);
    // Browser suite (needs build-admin fresh from this run; needs
    // `npx playwright install chromium` once per machine).
    run_or_exit("npm", &["run", "test:e2e", "--prefix", "web"]);
    // Fail-closed Tailwind audit (unknown utilities emit no CSS):
    // needs both builds fresh from this run.
    run_or_exit("npm", &["run", "check:tailwind", "--prefix", "web"]);
}

/// Run one CI step: missing binary is a loud fatal (never a silent
/// skip — that is how gates rot), a failing step propagates its code.
fn run_or_exit(cmd: &str, args: &[&str]) {
    let status = Command::new(cmd).args(args).status().unwrap_or_else(|e| {
        eprintln!("ci: cannot run {cmd}: {e} (is it installed?)");
        std::process::exit(1);
    });
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn run_init(args: &[String]) {
    let store = open_store(args);
    if let Err(e) = store.init() {
        eprintln!("init store dirs: {e}");
        std::process::exit(1);
    }
    let conn = open_db(&store);
    if let Err(e) = handbook_store::db::migrate(&conn) {
        eprintln!("migrate: {e}");
        std::process::exit(1);
    }
    println!("store ready: {}", store.root().display());
}

fn run_add_authority(args: &[String]) {
    let store = open_store(args);
    let conn = open_db(&store);
    let region = flag(args, "region").unwrap_or_default();
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let kind = required(args, "kind");
    let country = required(args, "country");
    let name = required(args, "name");
    let slug = required(args, "slug");
    let new = handbook_store::db::NewAuthority {
        kind: &kind,
        country: &country,
        region: &region,
        name: &name,
        slug: &slug,
        actor: &actor,
        at: unix_now(),
    };
    match handbook_store::db::add_authority(&conn, &new) {
        Ok(id) => println!("authority id: {id}"),
        Err(e) => {
            eprintln!("add-authority: {e}");
            std::process::exit(1);
        }
    }
}

/// The repository's top directory (`git rev-parse`), or the current
/// directory outside a checkout.
pub(crate) fn repo_root() -> std::path::PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&cwd)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().into())
        .unwrap_or(cwd)
}

/// Current unix time as i64 (audit `at` values).
pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before 1970")
        .as_secs()
        .try_into()
        .expect("unix time exceeds i64")
}

fn run_retention_sweep(args: &[String]) {
    let store = open_store(args);
    let conn = open_db(&store);
    let now = unix_now();
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    match handbook_store::db::retention_sweep(&conn, &store, now, &actor) {
        Ok(deleted) => println!("expired audio captures deleted: {deleted}"),
        Err(e) => {
            eprintln!("retention-sweep: {e}");
            std::process::exit(1);
        }
    }
}

fn run_analyze(args: &[String]) {
    use std::collections::HashMap;

    let store = open_store(args);
    let conn = open_db(&store);
    if let Err(e) = handbook_store::db::migrate(&conn) {
        eprintln!("migrate: {e}");
        std::process::exit(1);
    }
    if args.iter().any(|a| a == "--check") {
        return run_analyze_check(&conn);
    }
    if args.iter().any(|a| a == "--scan-captures") {
        return run_analyze_scan(&conn, &store, args);
    }
    let slug = required(args, "slug");
    let lang = flag(args, "lang").unwrap_or_else(|| "en".to_string());
    let raw_input = required(args, "input");
    let input = match raw_input.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("read input file: {e}");
            std::process::exit(1);
        }),
        None => raw_input,
    };
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    // Stub default (offline-safe); --live drafts via the Meta Model
    // API. The stub cites doc_id "fixture", so the CLI evidence IS
    // the fixture text in both modes.
    let client = draft_client(args, || live_client(&conn));
    let mut sources = HashMap::new();
    sources.insert("fixture".to_string(), input.clone());
    let req = handbook_agent::runner::RunRequest {
        slug: &slug,
        lang: &lang,
        input_text: &input,
        sources: &sources,
        actor: &actor,
        at: unix_now(),
        capture_id: handbook_agent::runner::UNATTRIBUTED_CAPTURE_ID,
    };
    match handbook_agent::runner::run_analysis(&conn, client.as_ref(), &req) {
        Ok(report) => {
            println!(
                "authority: {}\nproposed: {}\nrejected: {}\nsuppressed: {}\nskipped: {}",
                report.authority_slug,
                report.proposed,
                report.rejected,
                report.suppressed,
                report.skipped
            );
            for error in &report.errors {
                println!("rejected: {error}");
            }
        }
        Err(e) => {
            eprintln!("analyze: {e}");
            std::process::exit(1);
        }
    }
}

/// Drafting client for `analyze` and `eval`. `--replay <corpus>`
/// serves recorded Muse Spark responses offline (`cargo xtask judge`;
/// a request with no recording drafts nothing); `--live` drafts via
/// the Meta Model API, and `--live --record <corpus>` also appends
/// every response to a replay corpus; otherwise the offline stub.
pub(crate) fn draft_client(
    args: &[String],
    live: impl FnOnce() -> handbook_agent::model::MetaRestClient,
) -> Box<dyn contracts::model::ModelClient> {
    let is_live = args.iter().any(|a| a == "--live");
    let record = flag(args, "record");
    if let Some(corpus) = flag(args, "replay") {
        if is_live || record.is_some() {
            eprintln!("--replay is offline; it cannot combine with --live or --record");
            std::process::exit(2);
        }
        let client =
            handbook_agent::replay::ReplayClient::load(std::path::Path::new(&corpus), false)
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });
        eprintln!(
            "replay: {} recorded muse-spark-1.3 responses from {corpus}",
            client.len()
        );
        return Box::new(client);
    }
    // Live drafts go through the call cache: a rerun replays, and only
    // live calls count toward `meta.daily_call_cap`.
    let cached = || live().with_cache(call_cache::meta_cache(args));
    match (is_live, record) {
        (true, Some(corpus)) => Box::new(handbook_agent::replay::Recorder::new(
            cached(),
            std::path::PathBuf::from(corpus),
        )),
        (true, None) => Box::new(cached()),
        (false, Some(_)) => {
            eprintln!("--record needs --live (it records live responses)");
            std::process::exit(2);
        }
        (false, None) => Box::new(handbook_agent::model::StubClient),
    }
}

/// Live Meta client from sealed config, or exit with a human message.
/// Missing KEK / missing key / unreadable config all fail closed here —
/// `--live` never silently falls back to the stub (a stub run billed
/// as live would draft fixture records into real captures).
pub(crate) fn live_client(conn: &rusqlite::Connection) -> handbook_agent::model::MetaRestClient {
    // `$META_API_KEY` first (same rule as the atlas worker): staging and
    // one-shot runs without the admin-sealed key.
    if let Ok(key) = std::env::var("META_API_KEY")
        && !key.is_empty()
    {
        eprintln!("analyze: using $META_API_KEY");
        // Honor the sealed `meta.base_url` when sealed config loads;
        // the key still comes from the environment either way.
        let base = sealed_config::load_kek()
            .ok()
            .and_then(|(kek, _)| sealed_config::RuntimeConfig::load(conn, &kek).ok())
            .map(|(cfg, _)| cfg.meta_base_url)
            .filter(|url| !url.is_empty())
            .unwrap_or_else(|| handbook_agent::model::META_BASE_URL.to_string());
        return handbook_agent::model::MetaRestClient::new(base, key);
    }
    let (kek, _source) = sealed_config::load_kek().unwrap_or_else(|e| {
        eprintln!("analyze: {e}");
        std::process::exit(1);
    });
    let (cfg, warnings) = sealed_config::RuntimeConfig::load(conn, &kek).unwrap_or_else(|e| {
        eprintln!("analyze: config: {e}");
        std::process::exit(1);
    });
    for warning in &warnings {
        eprintln!("analyze: config warning: {warning}");
    }
    let Some(key) = cfg.meta_api_key else {
        eprintln!("analyze: meta.api_key is not set (admin config UI)");
        std::process::exit(2);
    };
    handbook_agent::model::MetaRestClient::new(cfg.meta_base_url, key.as_str().to_string())
}

/// `--check` probe: verify the sealed key against `GET /models` without
/// drafting anything. Exit 0 prints the catalog; anything else exits 1.
fn run_analyze_check(conn: &rusqlite::Connection) {
    let client = live_client(conn);
    match client.check() {
        Ok(ids) => println!("meta api ok: {}", ids.join(", ")),
        Err(e) => {
            eprintln!("analyze --check: {e}");
            std::process::exit(1);
        }
    }
}

/// Option-B scan mode: analyze captures with `id > watermark`, oldest
/// first, presenting each sidecar as the `fixture` source (byte-exact
/// citation checks preserved). A client error stops the run without
/// advancing past the failed capture (exit 1); the watermark only ever
/// moves inside a capture's own committed transaction.
fn run_analyze_scan(
    conn: &rusqlite::Connection,
    store: &handbook_store::store::Store,
    args: &[String],
) {
    use std::collections::HashMap;

    let client = draft_client(args, || live_client(conn));
    let slug = required(args, "slug");
    let lang = flag(args, "lang").unwrap_or_else(|| "en".to_string());
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let limit: i64 = match flag(args, "limit") {
        None => 25,
        Some(s) => s.parse().unwrap_or_else(|_| {
            eprintln!("analyze: bad --limit (positive integer)");
            std::process::exit(2);
        }),
    };
    let authority = authority_id(conn, &slug);
    let start = handbook_store::db::get_capture_watermark(conn, &slug).unwrap_or_else(|e| {
        eprintln!("analyze: watermark: {e}");
        std::process::exit(1);
    });
    let rows: Vec<(i64, i64)> = conn
        .prepare(
            "SELECT id, captured_at FROM capture
             WHERE authority_id = ?1 AND id > ?2 AND id > 0 AND trust != 'rejected'
             ORDER BY captured_at, id LIMIT ?3",
        )
        .unwrap_or_else(|e| {
            eprintln!("analyze: database error: {e}");
            std::process::exit(1);
        })
        .query_map(rusqlite::params![authority, start, limit], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap_or_else(|e| {
            eprintln!("analyze: database error: {e}");
            std::process::exit(1);
        })
        .collect::<rusqlite::Result<_>>()
        .unwrap_or_else(|e| {
            eprintln!("analyze: database error: {e}");
            std::process::exit(1);
        });
    let (mut proposed, mut rejected, mut skipped) = (0, 0, 0);
    let mut watermark = start;
    for (id, _) in &rows {
        let now = unix_now();
        let sidecar = store.root().join("text").join(format!("{id}.txt"));
        let text = match std::fs::read_to_string(&sidecar) {
            Ok(t) => t,
            Err(_) => {
                match handbook_agent::runner::skip_missing_sidecar(conn, &slug, *id, &actor, now) {
                    Ok(_) => {
                        println!("capture {id}: skipped (missing sidecar)");
                        skipped += 1;
                        watermark = *id;
                        continue;
                    }
                    Err(e) => {
                        eprintln!("analyze: capture {id}: {e}");
                        std::process::exit(1);
                    }
                }
            }
        };
        let mut sources = HashMap::new();
        sources.insert("fixture".to_string(), text.clone());
        let req = handbook_agent::runner::RunRequest {
            slug: &slug,
            lang: &lang,
            input_text: &text,
            sources: &sources,
            actor: &actor,
            at: now,
            capture_id: *id,
        };
        match handbook_agent::runner::run_analysis(conn, client.as_ref(), &req) {
            Ok(r) => {
                proposed += r.proposed;
                rejected += r.rejected;
                if r.skipped {
                    skipped += 1;
                }
                println!(
                    "capture {id}: proposed={} rejected={} skipped={}",
                    r.proposed, r.rejected, r.skipped
                );
                watermark = *id;
            }
            Err(e) => {
                eprintln!("analyze: capture {id}: {e} (watermark stays at {watermark})");
                std::process::exit(1);
            }
        }
    }
    println!(
        "analyze: {slug} scanned={} proposed={proposed} rejected={rejected} skipped={skipped} watermark={watermark}",
        rows.len()
    );
}

/// Authority id for a slug, or exit 1.
pub(crate) fn authority_id(conn: &rusqlite::Connection, slug: &str) -> i64 {
    use rusqlite::OptionalExtension as _;
    conn.query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |row| {
        row.get(0)
    })
    .optional()
    .unwrap_or_else(|e| {
        eprintln!("fetch-url: database error: {e}");
        std::process::exit(1);
    })
    .unwrap_or_else(|| {
        eprintln!("fetch-url: unknown authority slug: {slug}");
        std::process::exit(1);
    })
}

/// Find-or-create the `source` row for a watched URL.
fn ensure_source(conn: &rusqlite::Connection, authority: i64, kind: &str, uri: &str) -> i64 {
    use rusqlite::OptionalExtension as _;
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM source WHERE authority_id = ?1 AND uri = ?2",
            rusqlite::params![authority, uri],
            |row| row.get(0),
        )
        .optional()
        .unwrap_or_else(|e| {
            eprintln!("fetch-url: database error: {e}");
            std::process::exit(1);
        })
    {
        return id;
    }
    conn.execute(
        "INSERT INTO source(authority_id, kind, uri) VALUES (?1, ?2, ?3)",
        rusqlite::params![authority, kind, uri],
    )
    .unwrap_or_else(|e| {
        eprintln!("fetch-url: insert source: {e}");
        std::process::exit(1);
    });
    conn.last_insert_rowid()
}

/// Record a fetch failure as a `validate_fact` task (F1 acceptance 4):
/// insert, or refresh the live row's detail on dedup hit. Never silent.
pub(crate) fn record_fetch_failure(
    conn: &rusqlite::Connection,
    authority: i64,
    slug: &str,
    url: &str,
    failure: &str,
    actor: &str,
    at: i64,
) {
    let dedup_key = handbook_store::store::sha256_hex(format!("{slug}|fetch|{url}").as_bytes());
    let title = format!("fetch failed: {url}");
    ingest::sweep::record_task(
        conn,
        &ingest::sweep::NewTask {
            authority_id: authority,
            kind: "validate_fact",
            title: &title,
            detail: failure,
            priority: 0,
            dedup_key: &dedup_key,
            created_by: actor,
            created_at: at,
        },
    )
    .unwrap_or_else(|e| {
        eprintln!("fetch-url: insert task: {e}");
        std::process::exit(1);
    });
    println!("task: fetch failure recorded for {url}");
}

fn run_fetch_url(args: &[String]) {
    let store = open_store(args);
    let conn = open_db(&store);
    if let Err(e) = handbook_store::db::migrate(&conn) {
        eprintln!("migrate: {e}");
        std::process::exit(1);
    }
    let slug = required(args, "slug");
    let url = required(args, "url");
    let source_kind = flag(args, "source-kind").unwrap_or_else(|| "website_official".to_string());
    let trust = flag(args, "trust").unwrap_or_else(|| "official".to_string());
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let now = unix_now();

    let authority = authority_id(&conn, &slug);
    let source_id = ensure_source(&conn, authority, &source_kind, &url);

    // Robots gate for third-party hosts (F1): re-checked each crawl,
    // verdict persisted on the source row. A disallow files a task and
    // skips the fetch without an error exit.
    if source_kind == "website_thirdparty" {
        let verdict = ingest::fetch::check_robots(&url, std::time::Duration::from_secs(30));
        conn.execute(
            "UPDATE source SET robots = ?1 WHERE id = ?2",
            rusqlite::params![format!("{} at {now}", verdict.text), source_id],
        )
        .unwrap_or_else(|e| {
            eprintln!("fetch-url: record robots verdict: {e}");
            std::process::exit(1);
        });
        if !verdict.allowed {
            record_fetch_failure(
                &conn,
                authority,
                &slug,
                &url,
                "robots disallow",
                &actor,
                now,
            );
            println!("skipped (robots disallow): {url}");
            return;
        }
    }

    let fetched = match ingest::fetch::fetch_url(&url, std::time::Duration::from_secs(30)) {
        Ok(fetched) => fetched,
        Err(e) => {
            record_fetch_failure(&conn, authority, &slug, &url, &e.to_string(), &actor, now);
            std::process::exit(1);
        }
    };
    let text = ingest::capture::extract_text(&fetched.bytes);
    let outcome = ingest::capture::ingest_bytes(
        &conn,
        &store,
        &ingest::capture::IngestInput {
            authority_id: authority,
            source_id: Some(source_id),
            kind: "warc",
            trust: &trust,
            bytes: &fetched.bytes,
            text: &text,
            captured_at: now,
            actor: &actor,
            origin: &url,
        },
    )
    .unwrap_or_else(|e| {
        eprintln!("fetch-url: ingest: {e}");
        std::process::exit(1);
    });
    match outcome {
        ingest::capture::IngestOutcome::Created { capture_id } => {
            println!("capture: {capture_id} (status {})", fetched.status);
        }
        ingest::capture::IngestOutcome::Duplicate { capture_id } => {
            println!("duplicate: {capture_id} (unchanged bytes)");
        }
    }
}

/// NUL-separated paths from a git listing command.
fn git_paths(args: &[&str]) -> Vec<String> {
    let output = Command::new("git").args(args).output().unwrap_or_else(|e| {
        eprintln!("secrets-scan: git failed: {e}");
        std::process::exit(1);
    });
    if !output.status.success() {
        eprintln!("secrets-scan: git failed (not a repo?)");
        std::process::exit(1);
    }
    output
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect()
}

/// Scan tracked files, or `--staged` the index content (pre-commit).
/// Prints `path:line: rule` hits; exits 1 on any hit (fail closed).
fn run_secrets_scan(args: &[String]) {
    let staged = args.iter().any(|a| a == "--staged");
    let paths = if staged {
        git_paths(&[
            "diff",
            "--cached",
            "--name-only",
            "-z",
            "--no-renames",
            "--diff-filter=ACMR",
        ])
    } else {
        git_paths(&["ls-files", "-z"])
    };
    let mut hits = 0;
    for path in &paths {
        if path.contains("target/")
            || path.contains("node_modules/")
            || path.contains(".svelte-kit/")
        {
            continue;
        }
        let bytes = if staged {
            let spec = format!(":{path}");
            match Command::new("git").args(["show", &spec]).output() {
                Ok(o) if o.status.success() => o.stdout,
                _ => {
                    eprintln!("secrets-scan: cannot read staged {path}");
                    std::process::exit(1);
                }
            }
        } else {
            match std::fs::read(path) {
                Ok(b) => b,
                Err(_) => continue, // deleted after listing; ignore
            }
        };
        for hit in secrets::scan_bytes(path, &bytes) {
            println!("{}:{}: {}", hit.file, hit.line, hit.rule);
            hits += 1;
        }
    }
    if hits > 0 {
        eprintln!("secrets-scan: {hits} finding(s) — refusing");
        std::process::exit(1);
    }
    eprintln!(
        "secrets-scan: clean ({} file(s){})",
        paths.len(),
        if staged { ", staged" } else { "" }
    );
}

/// Non-Meta model ids and endpoints: no tracked non-doc file
/// may name one (organizer rule). Split literals so this file
/// does not flag itself: no half ever forms a full needle, and
/// the halves join at compile time, so the scan still matches
/// everywhere else.
const MODEL_NEEDLES: &[&str] = &[
    concat!("qw", "en"),
    concat!("gp", "t-"),
    concat!("clau", "de-"),
    concat!("gem", "ini"),
    concat!("whis", "per"),
    concat!("deep", "gram"),
    concat!("eleven", "labs"),
    concat!("edge", "-tts"),
    concat!("api.openai", ".com"),
    concat!("api.anthropic", ".com"),
    concat!("generativelanguage.googleapis", ".com"),
    concat!("transcription", "_engine"),
];

/// First banned needle on each offending line (pure: unit-tested).
fn model_gate_hits(path: &str, bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes).to_lowercase();
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if let Some(needle) = MODEL_NEEDLES.iter().find(|needle| line.contains(**needle)) {
            out.push(format!("{path}:{}: {needle}", n + 1));
        }
    }
    out
}

/// Organizer rule: every model called at runtime must be a Meta
/// model. Fail on non-Meta model ids or endpoints in tracked
/// non-doc files, printing each path. Docs (`*.md`) are exempt —
/// they may discuss the rule and its history.
fn run_model_gate() {
    let paths = git_paths(&["ls-files", "-z"]);
    let mut hits = 0;
    let mut scanned = 0;
    for path in &paths {
        if path.ends_with(".md")
            || path.contains("target/")
            || path.contains("node_modules/")
            || path.contains(".svelte-kit/")
        {
            continue;
        }
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue, // deleted after listing; ignore
        };
        scanned += 1;
        for hit in model_gate_hits(path, &bytes) {
            println!("{hit}");
            hits += 1;
        }
    }
    if hits > 0 {
        eprintln!("model-gate: {hits} non-meta model reference(s) — refusing");
        std::process::exit(1);
    }
    eprintln!("model-gate: clean ({scanned} file(s))");
}

/// Validate the pack tree at `--packs <dir>` (default: `packs`): the
/// curated packs plus `world/` and `world-nonparty/` when present,
/// with the name lint and one owner per slug across all of them.
/// Exit 1 lists every violation; exit 0 prints the clean count.
fn run_packs_validate(args: &[String]) {
    let dir = flag(args, "packs").unwrap_or_else(|| "packs".to_string());
    let root = std::path::Path::new(&dir);
    let errors = packs::validate_tree(root);
    if errors.is_empty() {
        let count = packs::tree_pack_count(root);
        println!("packs-validate: clean ({count} pack(s) under {dir})");
    } else {
        for error in &errors {
            println!("{error}");
        }
        eprintln!("packs-validate: {} pack(s) rejected", errors.len());
        std::process::exit(1);
    }
}

/// Load every validated pack into `--store <path>`. Refuses to load
/// when validation fails. Never writes `fact` or `capture` rows.
/// `--against <root>` validates `--packs` together with the pack tree
/// at `<root>` (a staged research set must not claim a release slug).
fn run_packs_load(args: &[String]) {
    let dir = flag(args, "packs").unwrap_or_else(|| "packs".to_string());
    let errors = match flag(args, "against") {
        Some(root) => {
            packs::validate_against(std::path::Path::new(&dir), std::path::Path::new(&root))
        }
        None => packs::validate_dir(std::path::Path::new(&dir)),
    };
    if !errors.is_empty() {
        for error in &errors {
            println!("{error}");
        }
        eprintln!("packs-load: refusing to load invalid packs");
        std::process::exit(1);
    }
    let store = open_store(args);
    if let Err(e) = store.init() {
        eprintln!("init store dirs: {e}");
        std::process::exit(1);
    }
    let conn = open_db(&store);
    if let Err(e) = handbook_store::db::migrate(&conn) {
        eprintln!("migrate: {e}");
        std::process::exit(1);
    }
    let actor = flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let now = unix_now();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| {
            eprintln!("packs-load: list {dir}: {e}");
            std::process::exit(1);
        })
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    entries.sort();
    let mut packs = 0;
    for path in entries {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            eprintln!("packs-load: read {}: {e}", path.display());
            std::process::exit(1);
        });
        // Validation above already parsed this file; a second parse
        // failure means the file changed mid-run — abort loudly.
        let file = packs::PackFile::parse(&bytes).unwrap_or_else(|e| {
            eprintln!("packs-load: re-parse {}: {e}", path.display());
            std::process::exit(1);
        });
        match packs::load_pack(&conn, &file, &actor, now) {
            Ok(report) => {
                println!(
                    "{}: +{} authorities ~{} updated -{} hidden +{} domains +{} sources \
                     +{} tasks -{} stale tasks +{} paths ~{} routes",
                    file.pack.pack,
                    report.authorities,
                    report.updated,
                    report.hidden,
                    report.domains,
                    report.sources,
                    report.tasks,
                    report.closed,
                    report.paths,
                    report.routes
                );
                packs += 1;
            }
            Err(e) => {
                eprintln!("packs-load: {}: {e}", path.display());
                std::process::exit(1);
            }
        }
    }
    println!(
        "packs-load: {packs} pack(s) into {}",
        store.root().display()
    );
}

/// Point this clone's git at the tracked `.githooks/` dir.
fn run_install_hooks() {
    let status = Command::new("git")
        .args(["config", "core.hooksPath", ".githooks"])
        .status()
        .unwrap_or_else(|e| {
            eprintln!("install-hooks: git failed: {e}");
            std::process::exit(1);
        });
    if !status.success() {
        eprintln!("install-hooks: git config failed");
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let hook = std::path::Path::new(".githooks/pre-commit");
        let mut perms = std::fs::metadata(hook)
            .unwrap_or_else(|e| {
                eprintln!("install-hooks: read {}: {e}", hook.display());
                std::process::exit(1);
            })
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(hook, perms).unwrap_or_else(|e| {
            eprintln!("install-hooks: chmod {}: {e}", hook.display());
            std::process::exit(1);
        });
    }
    println!("hooks installed: core.hooksPath=.githooks");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_gate_flags_banned_ids_and_passes_meta_ones() {
        // Build banned strings at runtime: literals here would trip
        // the gate on its own source.
        let model = ["Qw", "en2.5-VL"].concat();
        let host = ["api.openai", ".com"].concat();
        let src = format!("let m = \"muse-spark-1.3\";\nlet q = \"{model}\";\ncall {host}/v1\n");
        let hits = model_gate_hits("crates/x.rs", src.as_bytes());
        assert_eq!(hits.len(), 2, "got: {hits:?}");
        assert!(hits[0].ends_with(MODEL_NEEDLES[0]), "got: {}", hits[0]);
        assert!(hits[1].contains(MODEL_NEEDLES[8]), "got: {}", hits[1]);
        assert!(model_gate_hits("crates/y.rs", b"muse-voice-transcribe-1.0\nsam-3.1\n").is_empty());
    }
}
