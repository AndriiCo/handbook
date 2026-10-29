//! Project task runner. Use `cargo xtask <task>` instead of shell scripts.
//!
//! Keeping tasks in Rust reuses the project's language, libraries, and
//! error handling instead of scattering logic across shell dialects.

use std::process::Command;

use handbook_store::store::Store;

mod accept;
mod admin_cert;
mod admin_staff;
mod build_handbook;
mod config;
mod demo_vietnam;
mod fetch_slug;
mod form_scaffold;
mod mcp_serve;
mod packs;
mod pii_sweep;
mod quic_probe;
mod release;
mod review_conflicts;
mod secrets;
mod voice_replay;
mod watch;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("--help" | "-h" | "help") => print_help(),
        Some("ci") => run_ci(),
        Some("init") => run_init(&args[1..]),
        Some("add-authority") => run_add_authority(&args[1..]),
        Some("retention-sweep") => run_retention_sweep(&args[1..]),
        Some("web-budget") => run_web_budget(),
        Some("serve") => run_serve(),
        Some("analyze") => run_analyze(&args[1..]),
        Some("fetch-url") => run_fetch_url(&args[1..]),
        Some("fetch-slug") => fetch_slug::run(&args[1..]),
        Some("watch-sweep") => watch::run_sweep(&args[1..]),
        Some("hcch-poll") => watch::run_hcch(&args[1..]),
        Some("mail-poll") => watch::run_mail(&args[1..]),
        Some("review-conflicts") => review_conflicts::run(&args[1..]),
        Some("build-handbook") => build_handbook::run(&args[1..]),
        Some("mcp-serve") => mcp_serve::run(&args[1..]),
        Some("pii-sweep") => pii_sweep::run(&args[1..]),
        Some("accept") => accept::run(&args[1..]),
        Some("form-scaffold") => form_scaffold::run(&args[1..]),
        Some("quic-probe") => quic_probe::run(&args[1..]),
        Some("demo-vietnam") => demo_vietnam::run(&args[1..]),
        Some("packs-validate") => run_packs_validate(&args[1..]),
        Some("packs-load") => run_packs_load(&args[1..]),
        Some("config-enroll") => config::run_enroll(&args[1..]),
        Some("config-set") => config::run_set(&args[1..]),
        Some("config-get") => config::run_get(&args[1..]),
        Some("config-verify") => config::run_verify(&args[1..]),
        Some("config-rotate") => config::run_rotate(&args[1..]),
        Some("setup-key-mint") => config::run_setup_key_mint(&args[1..]),
        Some("admin-cert-issue") => admin_cert::run_issue(&args[1..]),
        Some("admin-cert-verify") => admin_cert::run_verify(&args[1..]),
        Some("admin-cert-revoke") => admin_cert::run_revoke(&args[1..]),
        Some("admin-invite") => admin_staff::run_invite(&args[1..]),
        Some("admin-offboard") => admin_staff::run_offboard(&args[1..]),
        Some("secrets-scan") => run_secrets_scan(&args[1..]),
        Some("voice-replay") => voice_replay::run(&args[1..]),
        Some("deploy") => release::run_deploy(&args[1..]),
        Some("rollback") => release::run_rollback(&args[1..]),
        Some("install-hooks") => run_install_hooks(),
        Some("audit") => run_audit(),
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
  web-budget  report web/build JS+CSS size (advisory, no gate)
  serve       run the dev API server (cargo run -p web)
  analyze     draft proposed facts for an authority (stub default, --live for Meta API)
  fetch-url   fetch one URL into a warc capture (thin watch)
  fetch-slug  sweep one authority's watch URLs (manual, no scheduler)
  watch-sweep poll due watch URLs across authorities (timer runs this)
  hcch-poll   poll the HCCH status table, task on change (timer runs this)
  mail-poll   poll Gmail into email captures (timer runs this)
  review-conflicts print F6 conflicts with citations
  build-handbook accepted-only static build + sitemap/robots [--out --base-url]
  mcp-serve   serve lookup_authority on 127.0.0.1 [--port]
  pii-sweep   fail closed on PII in publishable surfaces
  accept      deploy gate: --expect-build must match latest build + facts_hash
  form-scaffold  per-form skeleton: --authority --code --title [--out] [--force]; --check compares
  quic-probe  HTTP/3 GET probe: --addr <ip:port> --server-name <sni> [--cacert <pem>] [--path <p>] [--host <h>]
  demo-vietnam  frozen VN drill on a temp store (never prod)
  packs-validate  validate packs/*.json (F13; needs_review invariants)
  packs-load  load packs into a store [--store <path>] (never writes facts)
  config-enroll|set|get|verify|rotate  encrypted config (see secure-config.md)
  setup-key-mint  mint the first-boot setup key (§9; prints once)
  admin-cert-issue|verify|revoke  admin mTLS client certs
  admin-invite  staff enrolment invite (spec customer-auth.md §5.1)
  admin-offboard  staff offboard: certs, sessions, creds, lock (§5.6)
  secrets-scan scan tracked files for secrets [--staged for the index]
  voice-replay  replay webhook fixtures at the stub (see --corpus/--scenario)
  install-hooks point git at .githooks/ (pre-commit secret scan)
  audit       cargo-audit over Cargo.lock (needs network; not in ci)
  deploy      ship a release (see flags below; runbook: docs/ops/runbook-release.md)
  rollback    roll back to a kept release (see flags below)

deploy flags:
  --version <sha>  release dir name, 7..=64 lowercase hex (required)
  --src <dir>   staged dir holding web + spa/ (required)
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
    run_or_exit("node", &["--test", "web/test/"]);
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

/// Advisory size report: raw JS+CSS bytes in `web/build` (top 8 +
/// total). History: this was a Day-1 budget gate (150KB, raised to
/// 200→220→221→223KB as P2 routes, form-framework, OAuth UI, and
/// magic-link UI landed). The author removed the gate on 2026-10-28:
/// JS/HTML/image sizes are unlimited until the initial-page
/// optimization pass; this command only reports, never fails on size.
fn run_web_budget() {
    let mut sized: Vec<(u64, String)> = Vec::new();
    let mut stack = vec![std::path::PathBuf::from("web/build")];
    if !stack[0].is_dir() {
        eprintln!("web-budget: web/build missing (run `npm run build` in web/ first)");
        std::process::exit(1);
    }
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| {
            eprintln!("web-budget: read {}: {e}", dir.display());
            std::process::exit(1);
        });
        for entry in entries {
            let entry = entry.unwrap_or_else(|e| {
                eprintln!("web-budget: entry: {e}");
                std::process::exit(1);
            });
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("js" | "css")
            ) {
                let bytes = entry
                    .metadata()
                    .unwrap_or_else(|e| {
                        eprintln!("web-budget: stat {}: {e}", path.display());
                        std::process::exit(1);
                    })
                    .len();
                sized.push((bytes, path.display().to_string()));
            }
        }
    }
    sized.sort();
    sized.reverse();
    let total: u64 = sized.iter().map(|(b, _)| b).sum();
    for (bytes, path) in sized.iter().take(8) {
        println!("{bytes:>8}  {path}");
    }
    println!("total JS+CSS: {total} bytes (advisory, no gate)");
}

fn run_serve() {
    let status = Command::new("cargo")
        .args(["run", "-p", "web"])
        .status()
        .expect("failed to run cargo");
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn run_analyze(args: &[String]) {
    use handbook_agent::model::StubClient;
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
    let client: Box<dyn contracts::model::ModelClient> = if args.iter().any(|a| a == "--live") {
        Box::new(live_client(&conn))
    } else {
        Box::new(StubClient)
    };
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

/// Live Meta client from sealed config, or exit with a human message.
/// Missing KEK / missing key / unreadable config all fail closed here —
/// `--live` never silently falls back to the stub (a stub run billed
/// as live would draft fixture records into real captures).
fn live_client(conn: &rusqlite::Connection) -> handbook_agent::model::MetaRestClient {
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
    use handbook_agent::model::StubClient;
    use std::collections::HashMap;

    let client: Box<dyn contracts::model::ModelClient> = if args.iter().any(|a| a == "--live") {
        Box::new(live_client(conn))
    } else {
        Box::new(StubClient)
    };
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

/// Validate every `*.json` pack in `--packs <dir>` (default: `packs`).
/// Exit 1 lists every violation; exit 0 prints the clean count.
fn run_packs_validate(args: &[String]) {
    let dir = flag(args, "packs").unwrap_or_else(|| "packs".to_string());
    let errors = packs::validate_dir(std::path::Path::new(&dir));
    if errors.is_empty() {
        let count = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok().map(|e| e.path()))
                    .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
                    .count()
            })
            .unwrap_or(0);
        println!("packs-validate: clean ({count} pack(s) in {dir})");
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
fn run_packs_load(args: &[String]) {
    let dir = flag(args, "packs").unwrap_or_else(|| "packs".to_string());
    let errors = packs::validate_dir(std::path::Path::new(&dir));
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
        let pack: packs::Pack = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            eprintln!("packs-load: re-parse {}: {e}", path.display());
            std::process::exit(1);
        });
        match packs::load_pack(&conn, &pack, &actor, now) {
            Ok(report) => {
                println!(
                    "{}: +{} authorities +{} domains +{} sources +{} tasks",
                    pack.pack, report.authorities, report.domains, report.sources, report.tasks
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

/// Surface `cargo audit` as a task. Deliberately NOT part of `ci`
/// (needs the network + the plugin); run before releases.
fn run_audit() {
    let status = Command::new("cargo").args(["audit"]).status();
    match status {
        Ok(s) if s.success() => println!("audit: clean"),
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("audit: cargo-audit not installed ({e}); run `cargo install cargo-audit`");
            std::process::exit(1);
        }
    }
}
