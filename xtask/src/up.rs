//! One-command local boot: `cargo xtask up [--live] [--fresh] [--check]`.
//!
//! Brings a fresh checkout to a running handbook on loopback: checks the
//! pinned toolchains, installs the web dependencies (skipped when the
//! lockfile is unchanged), builds both SPAs, builds the release
//! binaries, enrolls a file KEK with a DEV CRYPTO banner, seals the
//! loopback config, loads every pack, ingests one capture per gold case
//! from `evals/authorities/*.json` (synthetic evidence) plus one
//! stub-fixture capture per authority, and drafts proposals with
//! `analyze --scan-captures`, installs the recorded A2 verdicts,
//! grades `eval` into `quality.json`, mints the setup key, then serves
//! the release `web` binary in the foreground.
//!
//! `--live` swaps the synthetic captures for `fetch-slug` runs and
//! drafts/scores through the Meta Model API (`META_API_KEY`).
//! `--fresh` wipes `.local/` first. `--check` boots, probes the health,
//! authorities, and setup-status endpoints, then exits without serving.
//! `--from-archive <url|dir> [--country ISO2]` additionally imports
//! response records from a crawl package (sha256-verified against its
//! SHA256SUMS) as `kind='warc'` captures for authorities the loaded
//! packs already know; records with no matching source URI are skipped.
//!
//! Release binaries never see real credentials here: the KEK is a fresh
//! local file key, the evidence is synthetic, and every bind stays on
//! loopback (the release dev-seam gate refuses anything else).

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// Local store root (git-ignored scratch, never committed).
const STORE_DIR: &str = ".local/store";
/// Raw 32-byte file KEK enrolled by `config-enroll` (`--kek-file` form).
const KEK_FILE: &str = ".local/handbook-kek";
/// Remembers the lockfile hash the last `npm ci` installed.
const NCI_HASH_FILE: &str = ".local/.npm-ci-hash";
/// Default ports match the staging runbook (operator forwards these
/// over SSH); `--port-base N` serves public on N and admin on N+1.
const DEFAULT_PORT_BASE: u16 = 18318;
/// Synthetic A2 verdicts committed for local boots.
const VERDICTS_SRC: &str = "evals/captures/demo_precheck";

/// Environment label the served binary reports on the admin env badge
/// (`APOSTILLE_ENV_LABEL`): `local` for `up`, `judge` for `xtask judge`.
static ENV_LABEL: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// `xtask judge` sets `judge` before calling [`run`] (first set wins).
pub fn set_env_label(label: &'static str) {
    let _ = ENV_LABEL.set(label);
}

fn env_label() -> &'static str {
    ENV_LABEL.get().copied().unwrap_or("local")
}

/// Public port; admin is the next one (`--port-base`, first set wins).
static PORT_BASE: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

pub fn public_port() -> u16 {
    PORT_BASE.get().copied().unwrap_or(DEFAULT_PORT_BASE)
}

pub fn admin_port() -> u16 {
    public_port() + 1
}

/// Read `--port-base N` (1024-65534, so N+1 is a valid port) and
/// remember it; `xtask judge` calls this before its doctor.
pub fn apply_port_base(args: &[String]) -> Result<(), String> {
    let Some(i) = args.iter().position(|a| a == "--port-base") else {
        return Ok(());
    };
    let base = args
        .get(i + 1)
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|b| (1024..=65534).contains(b))
        .ok_or("--port-base needs a port from 1024 to 65534")?;
    let _ = PORT_BASE.set(base);
    Ok(())
}

/// Entry point (the builder wires the `up` arm in `main.rs`).
pub fn run(args: &[String]) {
    let live = has(args, "--live");
    let fresh = has(args, "--fresh");
    let check = has(args, "--check");
    let archive = archive_arg(args).unwrap_or_else(|| usage());
    check_flags(args);
    apply_port_base(args).unwrap_or_else(|e| fail(&e));
    assert_repo_root();
    check_toolchain();
    if fresh && Path::new(".local").exists() {
        println!("up: --fresh: removing .local");
        std::fs::remove_dir_all(".local").unwrap_or_else(|e| fail(&format!("rm .local: {e}")));
    }
    npm_ci_if_needed();
    npm_build("build");
    npm_build("build:admin");
    cargo_build_release();
    let xtask = release_bin("xtask");
    let web = release_bin("web");
    xtask_cmd(&xtask, &["init", "--store", STORE_DIR]);
    enroll_if_needed(&xtask);
    config_set_all(&xtask);
    xtask_cmd(
        &xtask,
        &[
            "config-verify",
            "--store",
            STORE_DIR,
            "--kek-file",
            KEK_FILE,
        ],
    );
    xtask_cmd(&xtask, &["packs-load", "--store", STORE_DIR]);
    let slugs = gold_slugs();
    if live {
        for slug in &slugs {
            xtask_cmd(
                &xtask,
                &["fetch-slug", "--store", STORE_DIR, "--slug", slug],
            );
        }
    } else {
        seed_captures(&slugs);
    }
    if let Some((src, country)) = &archive {
        import_from_archive(src, country.as_deref());
    }
    for slug in &slugs {
        let mut step = vec![
            "analyze".to_string(),
            "--store".to_string(),
            STORE_DIR.to_string(),
            "--slug".to_string(),
            slug.to_string(),
            "--scan-captures".to_string(),
        ];
        if live {
            step.push("--live".to_string());
        }
        xtask_cmd_ref(&xtask, &step);
    }
    install_verdicts();
    if live {
        xtask_cmd(&xtask, &["eval", "--store", STORE_DIR, "--live"]);
    } else {
        xtask_cmd(&xtask, &["eval", "--store", STORE_DIR]);
        install_recorded_quality();
    }
    let setup_key = setup_key_mint(&xtask);
    let cfg = load_runtime_config();
    let review_link = match staff_review_link(cfg.bind, cfg.bind6, admin_port()) {
        Ok(link) => link,
        Err(e) => fail(&format!("staff login link refused: {e}")),
    };
    if check {
        run_check(&web, &setup_key, &review_link);
        return;
    }
    print_banner(&setup_key, &review_link);
    serve_foreground(&web);
}

fn usage() -> ! {
    eprintln!(
        "usage: cargo xtask up [--live] [--fresh] [--check] [--port-base N] [--from-archive <url|dir> [--country ISO2]]"
    );
    std::process::exit(2);
}

/// Value-taking flags. `--from-archive` takes the package URL or
/// directory; `--country` takes an ISO2 code and requires
/// `--from-archive`. Unknown `--` flags fail closed in
/// [`check_flags`]; a missing value here is a usage error.
fn archive_arg(args: &[String]) -> Option<Option<(String, Option<String>)>> {
    let mut src: Option<String> = None;
    let mut country: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--from-archive" => {
                i += 1;
                src = args.get(i).cloned();
                if src.as_deref().is_none_or(|s| s.starts_with("--")) {
                    return None;
                }
            }
            "--country" => {
                i += 1;
                country = args.get(i).cloned();
                if country.as_deref().is_none_or(|s| s.starts_with("--")) {
                    return None;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if country.is_some() && src.is_none() {
        return None;
    }
    Some(src.map(|s| (s, country)))
}

fn check_flags(args: &[String]) {
    const VALUE_FLAGS: [&str; 3] = ["--from-archive", "--country", "--port-base"];
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if VALUE_FLAGS.contains(&arg.as_str()) {
            skip_next = true;
            continue;
        }
        if arg.starts_with("--") && !["--live", "--fresh", "--check"].contains(&arg.as_str()) {
            usage();
        }
    }
}

fn has(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn fail(message: &str) -> ! {
    eprintln!("up: {message}");
    std::process::exit(1);
}

/// Refuse to run anywhere but the repo root (every path below is root
/// relative: `web/`, `packs/`, `evals/`, `.local/`).
fn assert_repo_root() {
    for marker in [
        "Cargo.toml",
        "web/package.json",
        "packs",
        "evals/authorities",
    ] {
        if !Path::new(marker).exists() {
            fail(&format!("run from the repo root (missing {marker})"));
        }
    }
}

/// Minimum accepted Node major (SA-11: judges arrive on Node 24 LTS,
/// 25, or the pinned 26 — all build the SPAs. Anything older fails
/// closed: a build under Node 22 or earlier is never "close enough").
const NODE_MIN_MAJOR: u64 = 24;

/// Minimum accepted npm major (`web/package.json` engines: npm >= 11).
/// Older npm falls back to `npm ci --legacy-peer-deps` with a printed
/// note instead of failing the boot.
const NPM_MIN_MAJOR: u64 = 11;

/// Toolchain pin check: `rustc` must carry the `rust-toolchain.toml`
/// channel; `node` must be >= [`NODE_MIN_MAJOR`] (the `.nvmrc` pin
/// names the preferred major and is still required to be present and
/// sane). A mismatch fails closed — a build under the wrong toolchain
/// is never "close enough".
fn check_toolchain() {
    let channel = rust_channel();
    let out = command_output("rustc", &["--version"]);
    if !rust_matches(&channel, &out) {
        fail(&format!(
            "rustc {out:?} does not match rust-toolchain.toml channel {channel:?}"
        ));
    }
    let want = node_want();
    let out = command_output("node", &["--version"]);
    if !node_matches(&want, &out) {
        fail(&format!(
            "node {out:?} is below the Node >={NODE_MIN_MAJOR} floor (.nvmrc prefers {want:?}); fix: install Node 24+ (nvm install 24)"
        ));
    }
    println!(
        "up: toolchain ok (rust {channel}, node {out}; floor Node >={NODE_MIN_MAJOR}, .nvmrc prefers {want})"
    );
}

fn rust_channel() -> String {
    let text = std::fs::read_to_string("rust-toolchain.toml")
        .unwrap_or_else(|e| fail(&format!("read rust-toolchain.toml: {e}")));
    let table: toml::Table =
        toml::from_str(&text).unwrap_or_else(|e| fail(&format!("parse rust-toolchain.toml: {e}")));
    table
        .get("toolchain")
        .and_then(|t| t.get("channel"))
        .and_then(|c| c.as_str())
        .unwrap_or_else(|| fail("rust-toolchain.toml: missing toolchain.channel"))
        .to_string()
}

fn node_want() -> String {
    let text =
        std::fs::read_to_string(".nvmrc").unwrap_or_else(|e| fail(&format!("read .nvmrc: {e}")));
    let want = text.trim().trim_start_matches('v').to_string();
    if want.is_empty() {
        fail(".nvmrc is empty");
    }
    want
}

/// Pure: `rustc --version` output carries the pinned channel.
fn rust_matches(channel: &str, rustc_version: &str) -> bool {
    rustc_version.contains(channel)
}

/// Pure: numeric major of a `node --version` / `.nvmrc` string
/// (`v26.10.0` → 26, `26` → 26). `None` on garbage — unparseable
/// versions fail closed at the call site.
fn node_major(version: &str) -> Option<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse::<u64>()
        .ok()
}

/// Pure: `node --version` output (`v26.10.0`) is accepted when its
/// major is >= [`NODE_MIN_MAJOR`] (24 LTS, 25, the pinned 26, later).
/// The `.nvmrc` pin must still parse (a garbage pin fails closed);
/// anything older than the floor — Node 22/20/18 — is refused.
fn node_matches(want: &str, node_version: &str) -> bool {
    if node_major(want).is_none() {
        return false;
    }
    node_major(node_version).is_some_and(|got| got >= NODE_MIN_MAJOR)
}

/// Pure: numeric major of `npm --version` output (`11.19.1` → 11).
/// `None` on garbage.
fn npm_major(npm_version: &str) -> Option<u64> {
    npm_version.trim().split('.').next()?.parse::<u64>().ok()
}

/// Pure: whether this npm needs the `--legacy-peer-deps` fallback.
/// `None` when the version is unparseable (fail closed at the call
/// site); `Some(false)` on npm >= [`NPM_MIN_MAJOR`].
fn npm_needs_legacy(npm_version: &str) -> Option<bool> {
    npm_major(npm_version).map(|got| got < NPM_MIN_MAJOR)
}

/// Run one command in the foreground; exit 1 with the failing line.
fn run_cmd(cmd: &str, args: &[&str]) {
    println!("up: + {cmd} {}", args.join(" "));
    let status = Command::new(cmd)
        .args(args)
        .status()
        .unwrap_or_else(|e| fail(&format!("spawn {cmd}: {e}")));
    if !status.success() {
        fail(&format!("{cmd} exited {}", status.code().unwrap_or(-1)));
    }
}

fn command_output(cmd: &str, args: &[&str]) -> String {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .unwrap_or_else(|e| fail(&format!("spawn {cmd}: {e}")));
    if !out.status.success() {
        fail(&format!("{cmd} exited {}", out.status.code().unwrap_or(-1)));
    }
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// `npm ci` unless the lockfile hash matches the last install.
/// `node_modules/` is git-ignored scratch; the hash file beside the
/// store makes reruns cheap without ever serving stale dependencies.
fn npm_ci_if_needed() {
    let lock = std::fs::read("web/package-lock.json")
        .unwrap_or_else(|e| fail(&format!("read web/package-lock.json: {e}")));
    let hash = handbook_store::store::sha256_hex(&lock);
    let installed = std::fs::read_to_string(NCI_HASH_FILE)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if installed == hash && Path::new("web/node_modules").is_dir() {
        println!("up: npm ci skipped (lockfile unchanged)");
        return;
    }
    // `web/package.json` engines want npm >= 11. On npm 10 the boot
    // still works: print the one-line fix, then fall back to
    // `--legacy-peer-deps` with a note instead of failing the judge.
    let npm_version = command_output("npm", &["--version"]);
    match npm_needs_legacy(&npm_version) {
        Some(false) => run_cmd("npm", &["ci", "--prefix", "web"]),
        Some(true) => {
            println!(
                "up: npm {npm_version} predates the engines floor (npm >={NPM_MIN_MAJOR}); one-line fix: npm install -g npm@{NPM_MIN_MAJOR}"
            );
            println!(
                "up: continuing with `npm ci --legacy-peer-deps` (npm {npm_version} fallback)"
            );
            run_cmd("npm", &["ci", "--legacy-peer-deps", "--prefix", "web"]);
        }
        None => fail(&format!("npm --version gave garbage {npm_version:?}")),
    }
    if let Some(parent) = Path::new(NCI_HASH_FILE).parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| fail(&format!("mkdir .local: {e}")));
    }
    std::fs::write(NCI_HASH_FILE, format!("{hash}\n"))
        .unwrap_or_else(|e| fail(&format!("write {NCI_HASH_FILE}: {e}")));
}

fn npm_build(script: &str) {
    run_cmd("npm", &["--prefix", "web", "run", script]);
}

fn cargo_build_release() {
    run_cmd("cargo", &["build", "--release", "-p", "web", "-p", "xtask"]);
}

/// Release binary path, honoring a shared `CARGO_TARGET_DIR`.
fn release_bin(name: &str) -> PathBuf {
    let dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
    let bin = PathBuf::from(dir).join("release").join(name);
    if !bin.is_file() {
        fail(&format!(
            "missing {} (release build did not emit it)",
            bin.display()
        ));
    }
    bin
}

/// Run the release `xtask` binary for one step (its stdout stays
/// visible: enroll banners, per-pack counts, eval rates).
fn xtask_cmd(bin: &Path, args: &[&str]) {
    println!("up: + {} {}", bin.display(), args.join(" "));
    let status = Command::new(bin)
        .args(args)
        .status()
        .unwrap_or_else(|e| fail(&format!("spawn {}: {e}", bin.display())));
    if !status.success() {
        fail(&format!(
            "{} {} exited {}",
            bin.display(),
            args.join(" "),
            status.code().unwrap_or(-1)
        ));
    }
}

fn xtask_cmd_ref(bin: &Path, args: &[String]) {
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    xtask_cmd(bin, &borrowed);
}

/// Enroll the file KEK once (`config-enroll` prints the DEV CRYPTO
/// banner itself). Reruns keep the existing key: rotating under a
/// running local store is a deliberate operator act, not a default.
fn enroll_if_needed(xtask: &Path) {
    if Path::new(KEK_FILE).exists() {
        println!("up: KEK exists ({KEK_FILE}); keeping it");
        return;
    }
    xtask_cmd(
        xtask,
        &[
            "config-enroll",
            "--store",
            STORE_DIR,
            "--kek-file",
            KEK_FILE,
        ],
    );
}

/// Seal the loopback config: 127.0.0.1 binds, local ports, the staff
/// WebAuthn origin on the admin port, the freshly built SPA/static
/// directories, and the local-dev admin seam. `auth.origin` is the
/// staff WebAuthn origin (the wizard says so: it must describe the
/// admin origin exactly, or enrol/log-in ceremonies fail) — the
/// WebAuthn handle admits this origin (plus the production admin
/// host), so it must be the loopback admin port for staff passkeys
/// to verify there. Loopback-only: the header-level origin gate
/// (`origin_allowed`) admits http localhost/127.0.0.1 solely under
/// the dev seam, and the staff link below is refused past loopback.
/// `admin.mtls_off` is what lets the admin host guard accept loopback
/// browsers; the release dev-seam gate only honors it while every
/// bind stays loopback.
fn config_set_all(xtask: &Path) {
    let (public, admin) = (public_port().to_string(), admin_port().to_string());
    let origin = format!("http://localhost:{admin}");
    let settings: [(&str, &str, &str); 9] = [
        ("net.bind", "text", "127.0.0.1"),
        ("net.port", "text", &public),
        ("net.admin_port", "text", &admin),
        ("auth.rp_id", "text", "localhost"),
        ("auth.origin", "text", &origin),
        ("paths.static_dir", "text", "crates/web/static"),
        ("paths.spa_dir", "text", "web/build"),
        ("paths.admin_spa_dir", "text", "web/build-admin"),
        ("admin.mtls_off", "bool", "true"),
    ];
    for (key, kind, value) in settings {
        xtask_cmd(
            xtask,
            &[
                "config-set",
                "--store",
                STORE_DIR,
                "--kek-file",
                KEK_FILE,
                "--key",
                key,
                "--type",
                kind,
                "--value",
                value,
            ],
        );
    }
}

/// Authority slugs in `evals/authorities/*.json` (sorted, stable).
fn gold_slugs() -> Vec<String> {
    let mut slugs = Vec::new();
    let entries = std::fs::read_dir("evals/authorities")
        .unwrap_or_else(|e| fail(&format!("read evals/authorities: {e}")));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| fail(&format!("list evals/authorities: {e}")));
        let path = entry.path();
        if !path.extension().is_some_and(|ext| ext == "json") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| fail(&format!("read {}: {e}", path.display())));
        let value: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| fail(&format!("parse {}: {e}", path.display())));
        let Some(slug) = value.get("authority").and_then(|a| a.as_str()) else {
            fail(&format!("{}: missing string \"authority\"", path.display()));
        };
        slugs.push(slug.to_string());
    }
    slugs.sort();
    if slugs.is_empty() {
        fail("evals/authorities holds no gold files");
    }
    slugs
}

/// The stub's two fixed values (its only grounded draft). Kept as a
/// named constant so the seeded text and this comment cannot drift.
const STUB_FIXTURE_EVIDENCE: &str = "Local handbook fixture (synthetic): the office mailing address is PO Box 40220 and counter hours are Mon-Fri 9-5. This paragraph exists so the offline stub has grounded values to draft; live runs draft from fetched pages instead.";

/// One capture per gold case, ingested in-process so the evidence text
/// lands byte-identical in the row, the blob, and the `text/<id>.txt`
/// sidecar that `analyze --scan-captures` drafts from. Dedup on
/// `(authority_id, sha256)` makes reruns no-ops.
///
/// Plus one stub-fixture capture per authority: the stub drafts only
/// its two fixed records, so gold evidence alone drafts nothing
/// storable and `/review` would stay empty. The fixture text carries
/// both values verbatim (the citation checker demands it) and is
/// labeled synthetic in its origin.
fn seed_captures(slugs: &[String]) {
    let store = handbook_store::store::Store::new(STORE_DIR);
    let conn =
        handbook_store::db::open(&store).unwrap_or_else(|e| fail(&format!("open store: {e}")));
    let now = unix_now();
    let mut created = 0;
    let mut dupes = 0;
    for slug in slugs {
        let authority = ensure_authority(&conn, slug, now);
        for (file, id, evidence) in gold_cases(slug) {
            ingest_one(
                &conn,
                &store,
                authority,
                evidence.as_bytes(),
                &evidence,
                &format!("synthetic gold {file}#{id}"),
                now,
                &mut created,
                &mut dupes,
            );
        }
        ingest_one(
            &conn,
            &store,
            authority,
            STUB_FIXTURE_EVIDENCE.as_bytes(),
            STUB_FIXTURE_EVIDENCE,
            &format!("synthetic stub-fixture evidence for {slug} (offline stub only)"),
            now,
            &mut created,
            &mut dupes,
        );
    }
    println!("up: captures: {created} created, {dupes} already present");
}

#[allow(clippy::too_many_arguments)]
fn ingest_one(
    conn: &rusqlite::Connection,
    store: &handbook_store::store::Store,
    authority: i64,
    bytes: &[u8],
    text: &str,
    origin: &str,
    now: i64,
    created: &mut u32,
    dupes: &mut u32,
) {
    let outcome = ingest::capture::ingest_bytes(
        conn,
        store,
        &ingest::capture::IngestInput {
            authority_id: authority,
            source_id: None,
            kind: "file",
            trust: "staff_verified",
            bytes,
            text,
            captured_at: now,
            actor: "xtask-up",
            origin,
        },
    )
    .unwrap_or_else(|e| fail(&format!("ingest {origin}: {e}")));
    match outcome {
        ingest::capture::IngestOutcome::Created { .. } => *created += 1,
        ingest::capture::IngestOutcome::Duplicate { .. } => *dupes += 1,
    }
}

/// `(file, case id, evidence)` for one gold authority.
fn gold_cases(slug: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir("evals/authorities")
        .unwrap_or_else(|e| fail(&format!("read evals/authorities: {e}")));
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    for path in files {
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| fail(&format!("read {}: {e}", path.display())));
        let value: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| fail(&format!("parse {}: {e}", path.display())));
        if value.get("authority").and_then(|a| a.as_str()) != Some(slug) {
            continue;
        }
        let file = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let cases = value
            .get("cases")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        for case in cases {
            let id = case
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or("?")
                .to_string();
            let Some(evidence) = case.get("evidence").and_then(|e| e.as_str()) else {
                fail(&format!("{file}#{id}: case without evidence text"));
            };
            if evidence.is_empty() {
                fail(&format!("{file}#{id}: empty evidence text"));
            }
            out.push((file.clone(), id, evidence.to_string()));
        }
    }
    if out.is_empty() {
        fail(&format!("no gold cases for authority {slug}"));
    }
    out
}

/// Row id for a gold slug, creating a loudly-logged bootstrap authority
/// when no pack defines it (SA-16's eval-only slugs `france-legalisation`
/// and `mexico-sre` have gold files but no packs). Judge-local `up` only:
/// the store lives in git-ignored `.local/`, the audit row names
/// `xtask-up`, and every seeded capture is labeled synthetic.
fn ensure_authority(conn: &rusqlite::Connection, slug: &str, now: i64) -> i64 {
    use rusqlite::OptionalExtension as _;
    let found: Option<i64> = conn
        .query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |row| {
            row.get(0)
        })
        .optional()
        .unwrap_or_else(|e| fail(&format!("authority lookup: {e}")));
    if let Some(id) = found {
        return id;
    }
    let (kind, country, name) = bootstrap_identity(slug);
    println!(
        "up: no pack defines authority {slug}; creating bootstrap authority (judge-local only)"
    );
    handbook_store::db::add_authority(
        conn,
        &handbook_store::db::NewAuthority {
            kind: &kind,
            country: &country,
            region: "",
            name: &name,
            slug,
            actor: "xtask-up",
            at: now,
        },
    )
    .unwrap_or_else(|e| fail(&format!("bootstrap authority {slug}: {e}")))
}

/// Mechanical display identity for a pack-less gold slug: kind is always
/// `office` (a guess scoped to judge-local bootstrap, never production),
/// country is the first slug token, name is the humanised slug. Short
/// words (<=3 chars) read as acronyms and stay uppercase.
fn bootstrap_identity(slug: &str) -> (String, String, String) {
    fn word(s: &str) -> String {
        if s.len() <= 3 {
            s.to_ascii_uppercase()
        } else {
            let mut c = s.chars();
            match c.next() {
                None => String::new(),
                Some(f) => format!("{f}{rest}", f = f.to_ascii_uppercase(), rest = c.as_str()),
            }
        }
    }
    let words: Vec<String> = slug.split('-').map(word).collect();
    let country = words.first().cloned().unwrap_or_else(|| slug.to_string());
    let name = words.join(" ");
    ("office".to_string(), country, name)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before 1970")
        .as_secs()
        .try_into()
        .expect("unix time exceeds i64")
}

/// Import crawl-package response records as `kind='warc'` captures.
/// `src` is a package directory or base URL; every `.warc.gz` under
/// its `warc/` tree is sha256-verified against the package SHA256SUMS
/// (fail closed when the sums file or an entry is missing) and each
/// 2xx response body is ingested for the authority whose loaded source
/// URI matches the record URL exactly (trailing slash aside). Records
/// with no matching authority are skipped and counted. Bodies are
/// stored as blobs through the normal ingest path, so `path` is the
/// blob address; the `warc/<file>#r<index>` provenance rides in the
/// audit `origin` (no schema change).
fn import_from_archive(src: &str, country: Option<&str>) {
    let pkg = package_dir(src);
    let sums = read_sums(&pkg);
    let files = warc_files(&pkg, country);
    if files.is_empty() {
        fail(&format!("no .warc.gz drops under {}", pkg.display()));
    }
    let store = handbook_store::store::Store::new(STORE_DIR);
    let conn =
        handbook_store::db::open(&store).unwrap_or_else(|e| fail(&format!("open store: {e}")));
    let sources = load_source_uris(&conn);
    let now = unix_now();
    let mut created = 0u32;
    let mut dupes = 0u32;
    let mut skipped = 0u32;
    let mut bytes = 0u64;
    for file in &files {
        let rel = file
            .strip_prefix(&pkg)
            .unwrap_or(file)
            .to_str()
            .unwrap_or("?")
            .to_string();
        verify_sha256(file, &rel, &sums);
        let records = ingest::warc::read_records(file)
            .unwrap_or_else(|e| fail(&format!("read {}: {e}", file.display())));
        for (idx, record) in records.iter().enumerate() {
            if !record_type_is(record, "response") {
                continue;
            }
            let Some(url) = record.header("WARC-Target-URI") else {
                skipped += 1;
                continue;
            };
            let body = http_body(&record.block);
            if body.is_none_or(|b| b.is_empty()) || !http_status_ok(&record.block) {
                skipped += 1;
                continue;
            }
            let body = body.unwrap_or_default();
            let Some(authority) = match_source(&sources, url) else {
                skipped += 1;
                continue;
            };
            let text = ingest::capture::extract_text(body);
            let origin = format!("warc:{rel}#r{idx} {url}");
            match ingest::capture::ingest_bytes(
                &conn,
                &store,
                &ingest::capture::IngestInput {
                    authority_id: authority,
                    source_id: None,
                    kind: "warc",
                    trust: "staff_verified",
                    bytes: body,
                    text: &text,
                    captured_at: now,
                    actor: "xtask-up",
                    origin: &origin,
                },
            )
            .unwrap_or_else(|e| fail(&format!("ingest {origin}: {e}")))
            {
                ingest::capture::IngestOutcome::Created { .. } => {
                    created += 1;
                    bytes += body.len() as u64;
                }
                ingest::capture::IngestOutcome::Duplicate { .. } => dupes += 1,
            }
        }
    }
    println!(
        "up: archive: {} file(s), {created} captures, {dupes} dupes, {skipped} skipped, {bytes} bytes",
        files.len()
    );
}

/// Resolve the package directory: a local dir is used as-is, otherwise
/// `src` is a base URL and every file named by its SHA256SUMS is
/// downloaded into `.local/archive/<host-path-tail>/` first.
fn package_dir(src: &str) -> PathBuf {
    let local = Path::new(src);
    if local.is_dir() {
        return local.to_path_buf();
    }
    let base = src.trim_end_matches('/');
    let dest: PathBuf = [
        ".local",
        "archive",
        &base
            .rsplit('/')
            .next()
            .unwrap_or("remote")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>(),
    ]
    .iter()
    .collect();
    std::fs::create_dir_all(&dest)
        .unwrap_or_else(|e| fail(&format!("mkdir {}: {e}", dest.display())));
    download(&format!("{base}/SHA256SUMS"), &dest.join("SHA256SUMS"));
    let text = std::fs::read_to_string(dest.join("SHA256SUMS"))
        .unwrap_or_else(|e| fail(&format!("read SHA256SUMS: {e}")));
    for (_, file) in parse_sums(&text) {
        let out = dest.join(&file);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|e| fail(&format!("mkdir {}: {e}", parent.display())));
        }
        download(&format!("{base}/{file}"), &out);
    }
    dest
}

fn download(url: &str, dest: &Path) {
    println!("up: + curl -fsSL {url}");
    let status = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "300",
            "--retry",
            "2",
            "-o",
            &dest.to_string_lossy(),
            url,
        ])
        .status()
        .unwrap_or_else(|e| fail(&format!("spawn curl: {e}")));
    if !status.success() {
        fail(&format!("download {url} failed"));
    }
}

/// Parse `SHA256SUMS` (`<64-hex> <path>` per line); junk lines never
/// read as valid entries.
fn parse_sums(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(hash), Some(file)) = (parts.next(), parts.next()) else {
            continue;
        };
        if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            out.push((hash.to_lowercase(), file.to_string()));
        }
    }
    out
}

fn read_sums(pkg: &Path) -> Vec<(String, String)> {
    let path = pkg.join("SHA256SUMS");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        fail(&format!(
            "{} is missing: packages verify against SHA256SUMS",
            path.display()
        ))
    });
    let sums = parse_sums(&text);
    if sums.is_empty() {
        fail(&format!("{} names no files", path.display()));
    }
    sums
}

/// `.warc.gz` drops under `<pkg>/warc/`, optionally limited to one
/// `warc/<ISO2>/` country directory (sorted for determinism).
fn warc_files(pkg: &Path, country: Option<&str>) -> Vec<PathBuf> {
    let root = pkg.join("warc");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| fail(&format!("read {}: {e}", dir.display())));
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "gz")
                && path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with(".warc.gz"))
            {
                out.push(path);
            }
        }
    }
    if let Some(cc) = country {
        let want = format!("warc/{}/", cc.to_uppercase());
        out.retain(|p| p.to_string_lossy().contains(&want as &str));
    }
    out.sort();
    out
}

fn verify_sha256(file: &Path, rel: &str, sums: &[(String, String)]) {
    let want = sums
        .iter()
        .find(|(_, f)| f == rel)
        .unwrap_or_else(|| fail(&format!("{rel} is not named in SHA256SUMS")))
        .0
        .clone();
    let bytes =
        std::fs::read(file).unwrap_or_else(|e| fail(&format!("read {}: {e}", file.display())));
    let got = handbook_store::store::sha256_hex(&bytes);
    if got != want {
        fail(&format!("sha256 mismatch for {rel}"));
    }
}

/// `(authority_id, uri)` for every loaded source (pure match over the
/// vec, so the matching rule unit-tests without a database).
fn load_source_uris(conn: &rusqlite::Connection) -> Vec<(i64, String)> {
    let mut stmt = conn
        .prepare("SELECT authority_id, uri FROM source")
        .unwrap_or_else(|e| fail(&format!("load sources: {e}")));
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap_or_else(|e| fail(&format!("load sources: {e}")))
        .filter_map(|r| r.ok())
        .collect()
}

/// Exact source-URI match, trailing slash aside.
fn match_source(sources: &[(i64, String)], url: &str) -> Option<i64> {
    sources
        .iter()
        .find(|(_, uri)| uri == url || uri.trim_end_matches('/') == url.trim_end_matches('/'))
        .map(|(id, _)| *id)
}

/// True when the record's `WARC-Type` is `kind` (case-insensitive).
fn record_type_is(record: &ingest::warc::WarcRecord, kind: &str) -> bool {
    record
        .header("WARC-Type")
        .is_some_and(|t| t.eq_ignore_ascii_case(kind))
}

/// HTTP status line of the response block is 2xx.
fn http_status_ok(block: &[u8]) -> bool {
    let line = match block.iter().position(|b| *b == b'\n') {
        Some(pos) => &block[..pos],
        None => block,
    };
    let line = std::str::from_utf8(line).unwrap_or("");
    let mut parts = line.split_whitespace();
    parts.next();
    parts
        .next()
        .is_some_and(|code| code.len() == 3 && code.starts_with('2'))
}

/// Body after the first blank line of the HTTP head; `None` when the
/// block carries no header terminator at all.
fn http_body(block: &[u8]) -> Option<&[u8]> {
    block
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|pos| &block[pos + 4..])
}

/// Recorded production eval (README "Eval numbers", 2026-11-01,
/// production, live key): the stub grades ~0% on the gold set because
/// its two fixture records cite `doc_id "fixture"`, never `"gold"`.
/// Serving that file would publish a 0% quality chip mislabeled as a
/// Muse Spark eval, so stub boots install these labelled recorded
/// numbers instead. `--live` keeps its fresh eval output untouched.
fn recorded_quality() -> serde_json::Value {
    serde_json::json!({
        "run_at": "2026-11-01T00:00:00Z",
        "gold_count": 26,
        "quote_found_rate": 1.0,
        "value_in_quote_rate": 0.969,
        "topic_accuracy": 0.641,
        "model": "muse-spark-1.3",
        "recorded_note": "recorded production eval, 2026-11-01",
    })
}

/// Overwrite the stub eval's `quality.json` with the recorded
/// production eval (see [`recorded_quality`]). The home-page chip
/// reads this file, so a stub boot shows the labelled recorded
/// numbers — never a 0% chip.
fn install_recorded_quality() {
    let dest = Path::new(STORE_DIR).join("quality.json");
    write_recorded_quality(&dest);
    println!(
        "up: stub eval replaced by recorded production eval, 2026-11-01 (home page never shows a 0% chip)"
    );
}

/// Pure write half of [`install_recorded_quality`] (split out so
/// tests can point it at a scratch file instead of the live store).
fn write_recorded_quality(dest: &Path) {
    let text = serde_json::to_string_pretty(&recorded_quality())
        .unwrap_or_else(|e| fail(&format!("encode recorded quality: {e}")));
    std::fs::write(dest, format!("{text}\n"))
        .unwrap_or_else(|e| fail(&format!("write {}: {e}", dest.display())));
}

/// Copy the recorded A2 verdicts into the store (the precheck endpoint
/// serves `<store>/demo_precheck/<sample>.json` verbatim).
fn install_verdicts() {
    let dest = Path::new(STORE_DIR).join("demo_precheck");
    std::fs::create_dir_all(&dest).unwrap_or_else(|e| fail(&format!("mkdir demo_precheck: {e}")));
    let mut count = 0;
    let mut entries: Vec<PathBuf> = std::fs::read_dir(VERDICTS_SRC)
        .unwrap_or_else(|e| fail(&format!("read {VERDICTS_SRC}: {e}")))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    entries.sort();
    for src in entries {
        let name = src.file_name().unwrap_or_default().to_owned();
        std::fs::copy(&src, dest.join(&name))
            .unwrap_or_else(|e| fail(&format!("install {}: {e}", src.display())));
        count += 1;
    }
    if count == 0 {
        fail(&format!("no verdicts in {VERDICTS_SRC}"));
    }
    println!("up: installed {count} recorded A2 verdict(s)");
}

/// Mint the first-boot setup key (prints once; the caller relays it to
/// the operator and never stores it elsewhere).
fn setup_key_mint(xtask: &Path) -> String {
    println!("up: + {} setup-key-mint", xtask.display());
    let out = Command::new(xtask)
        .args([
            "setup-key-mint",
            "--store",
            STORE_DIR,
            "--kek-file",
            KEK_FILE,
        ])
        .output()
        .unwrap_or_else(|e| fail(&format!("spawn setup-key-mint: {e}")));
    if !out.status.success() {
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        fail("setup-key-mint failed (staff passkeys exist? use --fresh for a clean local boot)");
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    parse_setup_key(&text).unwrap_or_else(|| fail("setup-key-mint printed no setup key"))
}

/// Pure: pull `setup_<64 hex>` out of the mint output.
fn parse_setup_key(text: &str) -> Option<String> {
    for token in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        if let Some(hex) = token.strip_prefix("setup_")
            && hex.len() == 64
            && hex.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Some(token.to_string());
        }
    }
    None
}

/// Read the sealed runtime config back with the file KEK (web boots
/// from the same rows; this is the pre-flight read, not a second
/// source of truth).
fn load_runtime_config() -> sealed_config::RuntimeConfig {
    let raw = std::fs::read(KEK_FILE).unwrap_or_else(|e| fail(&format!("read {KEK_FILE}: {e}")));
    if raw.len() != 32 {
        fail(&format!("{KEK_FILE}: want 32 raw bytes, got {}", raw.len()));
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&raw);
    let kek = sealed_config::Kek::from_bytes(bytes);
    let store = handbook_store::store::Store::new(STORE_DIR);
    let conn =
        handbook_store::db::open(&store).unwrap_or_else(|e| fail(&format!("open store: {e}")));
    match sealed_config::RuntimeConfig::load(&conn, &kek) {
        Ok((cfg, warnings)) => {
            for warning in warnings {
                eprintln!("up: config warning: {warning}");
            }
            cfg
        }
        Err(e) => fail(&format!("runtime config unreadable: {e}")),
    }
}

/// One-time staff login link for `/review`. Refused unless EVERY bind
/// IP is loopback — a login link is never printed for a host that
/// listens past localhost. The link always names `localhost`, never a
/// numeric loopback: the bind is proven loopback-only above, and every
/// printed URL in `up` uses the same host form judges type.
fn staff_review_link(
    bind: std::net::IpAddr,
    bind6: Option<std::net::IpAddr>,
    admin_port: u16,
) -> Result<String, String> {
    if !sealed_config::RuntimeConfig::dev_seam_allowed(true, bind, bind6) {
        return Err("every bind IP must be loopback (refusing to mint a staff link)".to_string());
    }
    Ok(format!("http://localhost:{admin_port}/review"))
}

/// Release `web` environment: the store plus the file KEK in
/// `dev:`-form. The binary runs in release, so the dev-seam gate
/// checks the sealed binds — loopback here, so it boots.
fn web_env() -> Vec<(String, String)> {
    let raw = std::fs::read(KEK_FILE).unwrap_or_else(|e| fail(&format!("read {KEK_FILE}: {e}")));
    vec![
        ("STORE_DIR".to_string(), STORE_DIR.to_string()),
        ("HANDBOOK_DEV_KEK".to_string(), format!("dev:{}", hex(&raw))),
        ("APOSTILLE_ENV_LABEL".to_string(), env_label().to_string()),
    ]
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0xf) as usize] as char);
    }
    out
}

fn spawn_web(web: &Path) -> Child {
    Command::new(web)
        .envs(web_env())
        .spawn()
        .unwrap_or_else(|e| fail(&format!("spawn {}: {e}", web.display())))
}

/// Minimal blocking HTTP GET over `TcpStream` (xtask has no HTTP
/// client; the probes need status + body only).
fn http_get(host: &str, port: u16, path: &str) -> Result<(u16, String), String> {
    use std::io::{Read as _, Write as _};
    use std::time::Duration;
    let addr = format!("{host}:{port}");
    let mut stream =
        std::net::TcpStream::connect(&addr).map_err(|e| format!("connect {addr}: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| format!("timeout: {e}"))?;
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .map_err(|e| format!("write: {e}"))?;
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .map_err(|e| format!("read: {e}"))?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| "bad status line".to_string())?;
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    Ok((status, body))
}

/// One probe; `Ok` prints the passing line, `Err` carries the reason.
/// Never exits: the caller stops the server first, so a failed check
/// can never orphan it with the terminal pipe held open.
fn probe(host: &str, port: u16, path: &str, want: &str) -> Result<(), String> {
    match http_get(host, port, path) {
        Ok((200, body)) if body.contains(want) => {
            println!("up: --check: GET {path} -> 200 (contains {want:?})");
            Ok(())
        }
        Ok((status, body)) => Err(format!(
            "GET {path} -> {status} (want 200 containing {want:?}): {}",
            body.chars().take(200).collect::<String>()
        )),
        Err(e) => Err(format!("GET {path}: {e}")),
    }
}

/// `--check`: boot the release binary, probe the three endpoints, stop
/// it, exit. Exercises the release dev-seam gate end to end (the boot
/// carries a dev KEK over loopback binds, plus the loopback-gated
/// `admin.mtls_off` seam the admin probes need).
fn run_check(web: &Path, setup_key: &str, review_link: &str) {
    let mut child = spawn_web(web);
    let mut failure: Option<String> = None;
    match wait_healthy(&mut child) {
        WaitOutcome::Healthy => {}
        WaitOutcome::Exited => {
            failure = Some("web exited before answering /healthz".to_string());
        }
        WaitOutcome::Timeout => {
            failure = Some("web did not answer /healthz in time".to_string());
        }
    }
    for (host, port, path, want) in [
        ("127.0.0.1", public_port(), "/apostille/healthz", "ok"),
        (
            "127.0.0.1",
            public_port(),
            "/apostille/api/v1/authorities",
            "authorities",
        ),
        (
            "127.0.0.1",
            admin_port(),
            "/apostille/api/v1/setup/status",
            "setup_required",
        ),
    ] {
        if failure.is_none()
            && let Err(e) = probe(host, port, path, want)
        {
            failure = Some(e);
        }
    }
    kill(&mut child);
    if let Some(e) = failure {
        fail(&e);
    }
    println!("up: --check: ok");
    println!("up: setup key (prints once): {setup_key}");
    println!("up: staff review link (loopback only): {review_link}");
}

/// Why the health wait ended (an enum, not a second `try_wait` at
/// the call site: the first reap consumes the status, so the reason
/// must travel with the return).
#[derive(Debug, PartialEq, Eq)]
enum WaitOutcome {
    Healthy,
    Exited,
    Timeout,
}

/// True when the child has already exited (reaps its status).
fn child_exited(child: &mut Child) -> bool {
    child.try_wait().ok().flatten().is_some()
}

/// Wait for `/healthz`, but stop the moment the server exits: a
/// crashed `web` used to burn the whole 180 s before `--check` could
/// report it. Split for tests: the deadline is a parameter so the
/// exited-child path proves it returns without waiting it out.
fn wait_healthy_until(child: &mut Child, deadline: std::time::Instant) -> WaitOutcome {
    wait_healthy_on(child, deadline, public_port())
}

/// Port-parameterized core: tests pass a provably closed port, so a
/// live server on the real public port (staging runs during the
/// plant gate) can never flip the outcome.
fn wait_healthy_on(child: &mut Child, deadline: std::time::Instant, port: u16) -> WaitOutcome {
    use std::time::Duration;
    loop {
        if child_exited(child) {
            println!("up: --check: web exited while waiting for /healthz");
            return WaitOutcome::Exited;
        }
        if std::time::Instant::now() >= deadline {
            return WaitOutcome::Timeout;
        }
        match http_get("127.0.0.1", port, "/apostille/healthz") {
            Ok((200, _)) => return WaitOutcome::Healthy,
            _ => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

fn wait_healthy(child: &mut Child) -> WaitOutcome {
    wait_healthy_until(
        child,
        std::time::Instant::now() + std::time::Duration::from_secs(180),
    )
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn print_banner(setup_key: &str, review_link: &str) {
    let (public, admin) = (public_port(), admin_port());
    println!();
    println!("handbook is up (loopback only, synthetic data):");
    println!("  public:  http://localhost:{public}/apostille");
    println!("  admin:   http://localhost:{admin}/");
    println!("  review:  {review_link}");
    println!("  setup key (prints ONCE — paste at /config to enrol staff):");
    println!("    {setup_key}");
    println!("try this:");
    println!("  - open the review queue and approve a proposed fact");
    println!("  - curl -s http://localhost:{public}/apostille/api/v1/authorities");
    println!("  - POST {{\"sample\":\"syn-doc-clean\"}} to /apostille/api/v1/demo/precheck");
    if env_label() == "judge" {
        crate::judge::print_tour(public, admin);
    }
    println!();
}

/// Foreground serve: the release binary inherits stdio and holds the
/// terminal (Ctrl-C stops it). Everything above already printed.
fn serve_foreground(web: &Path) {
    let mut child = spawn_web(web);
    let status = child
        .wait()
        .unwrap_or_else(|e| fail(&format!("wait web: {e}")));
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolchain_pins_match() {
        assert!(rust_matches(
            "1.99.0",
            "rustc 1.99.0 (b940084d7 2026-09-28)"
        ));
        assert!(!rust_matches("1.99.0", "rustc 1.78.0 (abc 2024-01-01)"));
        // SA-11 floor: Node >= 24 accepted (24 LTS, 25, pinned 26);
        // anything older — or unparseable — fails closed.
        assert!(node_matches("26", "v26.10.0\n"));
        assert!(node_matches("26", "v24.1.0"));
        assert!(node_matches("26", "v25.0.0"));
        assert!(node_matches("24", "v26.10.0"));
        assert!(!node_matches("26", "v22.0.0"));
        assert!(!node_matches("26", "v20.19.0"));
        assert!(!node_matches("26", "v18.20.8"));
        assert!(!node_matches("26", "not node at all"));
        assert!(!node_matches("26", ""));
        assert!(!node_matches("garbage", "v26.10.0"));
        assert!(!node_matches("", "v26.10.0"));
        assert_eq!(node_major("v26.10.0"), Some(26));
        assert_eq!(node_major("26"), Some(26));
        assert_eq!(node_major("v24.1.0\n"), Some(24));
        assert_eq!(node_major("garbage"), None);
    }

    #[test]
    fn npm_legacy_fallback_boundary() {
        assert_eq!(npm_needs_legacy("11.19.1"), Some(false));
        assert_eq!(npm_needs_legacy("11.0.0"), Some(false));
        assert_eq!(npm_needs_legacy("12.0.0"), Some(false));
        assert_eq!(npm_needs_legacy("10.9.0"), Some(true));
        assert_eq!(npm_needs_legacy("10.0.0"), Some(true));
        assert_eq!(npm_needs_legacy("9.8.1"), Some(true));
        assert_eq!(npm_needs_legacy("garbage"), None);
        assert_eq!(npm_needs_legacy(""), None);
    }

    #[test]
    fn recorded_quality_is_labelled_production() {
        let q = recorded_quality();
        assert_eq!(q["model"], "muse-spark-1.3");
        assert_eq!(q["gold_count"], 26);
        assert_eq!(q["quote_found_rate"], 1.0);
        assert_eq!(q["value_in_quote_rate"], 0.969);
        assert_eq!(q["topic_accuracy"], 0.641);
        assert_eq!(q["recorded_note"], "recorded production eval, 2026-11-01");
        // A 0% chip must be impossible from this document.
        assert!(q["quote_found_rate"].as_f64().unwrap_or(0.0) > 0.0);
        let dir = tempfile::tempdir().expect("scratch dir");
        let dest = dir.path().join("quality.json");
        write_recorded_quality(&dest);
        let round: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&dest).expect("read back"))
                .expect("valid json");
        assert_eq!(round, q);
        assert!(
            round["run_at"]
                .as_str()
                .unwrap_or("")
                .starts_with("2026-11-01")
        );
    }

    #[test]
    fn setup_key_parses() {
        let mint = "setup key (prints ONCE — paste into /config to start setup):\n  setup_abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\n";
        assert_eq!(
            parse_setup_key(mint).as_deref(),
            Some("setup_abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
        );
        assert_eq!(parse_setup_key("no key here"), None);
        assert_eq!(parse_setup_key("setup_short"), None);
    }

    #[test]
    fn archive_flags_parse() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(archive_arg(&args(&[])), Some(None));
        assert_eq!(
            archive_arg(&args(&["--check"])),
            Some(None),
            "other flags leave archive unset"
        );
        assert_eq!(
            archive_arg(&args(&["--from-archive", "archive/run-1"])),
            Some(Some(("archive/run-1".to_string(), None)))
        );
        assert_eq!(
            archive_arg(&args(&[
                "--from-archive",
                "https://x/y/",
                "--country",
                "vn"
            ])),
            Some(Some(("https://x/y/".to_string(), Some("vn".to_string()))))
        );
        assert_eq!(archive_arg(&args(&["--from-archive"])), None);
        assert_eq!(
            archive_arg(&args(&["--from-archive", "--check"])),
            None,
            "a flag is not a value"
        );
        assert_eq!(
            archive_arg(&args(&["--country", "US"])),
            None,
            "--country needs --from-archive"
        );
        // Valid inputs never trip the unknown-flag gate.
        check_flags(&args(&[]));
        check_flags(&args(&[
            "--check",
            "--from-archive",
            "archive/run-1",
            "--country",
            "US",
        ]));
    }

    #[test]
    fn bootstrap_identity_for_packless_slugs() {
        assert_eq!(
            bootstrap_identity("france-legalisation"),
            (
                "office".to_string(),
                "France".to_string(),
                "France Legalisation".to_string()
            )
        );
        assert_eq!(
            bootstrap_identity("mexico-sre"),
            (
                "office".to_string(),
                "Mexico".to_string(),
                "Mexico SRE".to_string()
            )
        );
        // Packed slugs never reach the bootstrap, but the shape holds anyway.
        assert_eq!(
            bootstrap_identity("usa-wa-sos"),
            (
                "office".to_string(),
                "USA".to_string(),
                "USA WA SOS".to_string()
            )
        );
    }

    #[test]
    fn sums_parse_strict() {
        let good = "a".repeat(64);
        let text =
            format!("{good}  warc/US/x-00000.warc.gz\njunk line\nshort  warc/ZZ/y.warc.gz\n");
        let sums = parse_sums(&text);
        assert_eq!(sums.len(), 1);
        assert_eq!(sums[0].1, "warc/US/x-00000.warc.gz");
        assert!(parse_sums("no hashes here\n").is_empty());
    }

    #[test]
    fn source_match_slash_insensitive() {
        let sources = vec![(7, "https://www.sos.wa.gov/apostille".to_string())];
        assert_eq!(
            match_source(&sources, "https://www.sos.wa.gov/apostille"),
            Some(7)
        );
        assert_eq!(
            match_source(&sources, "https://www.sos.wa.gov/apostille/"),
            Some(7)
        );
        assert_eq!(match_source(&sources, "https://example.com/"), None);
    }

    #[test]
    fn response_shape_checks() {
        let ok = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html>hi";
        assert!(http_status_ok(ok));
        assert_eq!(http_body(ok), Some(b"<html>hi".as_slice()));
        assert!(!http_status_ok(b"HTTP/1.1 404 Not Found\r\n\r\nnope"));
        assert!(!http_status_ok(b"not http at all"));
        assert_eq!(http_body(b"no headers here"), None);
        let rec = ingest::warc::WarcRecord {
            version: "WARC/1.1".to_string(),
            headers: vec![("WARC-Type".to_string(), "Response".to_string())],
            block: vec![],
        };
        assert!(record_type_is(&rec, "response"));
        assert!(!record_type_is(&rec, "request"));
    }

    #[test]
    fn staff_link_needs_loopback() {
        use std::net::{IpAddr, Ipv6Addr};
        let loopback = IpAddr::from([127, 0, 0, 1]);
        let lan: IpAddr = "192.168.1.2".parse().unwrap();
        // Every printed link names localhost — never a numeric loopback.
        assert_eq!(
            staff_review_link(loopback, None, 18319).as_deref(),
            Ok("http://localhost:18319/review")
        );
        assert_eq!(
            staff_review_link(loopback, Some(IpAddr::from(Ipv6Addr::LOCALHOST)), 18319).as_deref(),
            Ok("http://localhost:18319/review")
        );
        assert_eq!(
            staff_review_link(IpAddr::from(Ipv6Addr::LOCALHOST), None, 18319).as_deref(),
            Ok("http://localhost:18319/review")
        );
        assert!(staff_review_link(lan, None, 18319).is_err());
        assert!(staff_review_link(loopback, Some(lan), 18319).is_err());
    }

    #[test]
    fn port_base_rejects_ports_without_an_admin_neighbour() {
        let args = |v: &str| vec!["--port-base".to_string(), v.to_string()];
        assert!(apply_port_base(&args("65535")).is_err());
        assert!(apply_port_base(&args("80")).is_err());
        assert!(apply_port_base(&args("x")).is_err());
        assert!(apply_port_base(&["--port-base".to_string()]).is_err());
        assert!(apply_port_base(&[]).is_ok());
    }

    #[test]
    fn wait_stops_when_web_exits() {
        use std::process::Command;
        use std::time::{Duration, Instant};
        // An already-dead server must report Exited fast: the
        // deadline is long on purpose, so returning at all proves the
        // early exit (a deadline-only wait would hang 30 s here). The
        // probe port is freshly closed, so a live server on the real
        // port (staging serves during the plant gate) cannot flip it.
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind loopback")
            .local_addr()
            .expect("closed port")
            .port();
        let mut dead = Command::new("sh")
            .args(["-c", "exit 1"])
            .spawn()
            .expect("spawn sh");
        assert_eq!(
            wait_healthy_on(&mut dead, Instant::now() + Duration::from_secs(30), closed),
            WaitOutcome::Exited
        );
    }
}
