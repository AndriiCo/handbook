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
/// Ports match the staging runbook (operator forwards these over SSH).
const PUBLIC_PORT: u16 = 18318;
const ADMIN_PORT: u16 = 18319;
/// Synthetic A2 verdicts committed for local boots.
const VERDICTS_SRC: &str = "evals/captures/demo_precheck";

/// Entry point (the builder wires the `up` arm in `main.rs`).
pub fn run(args: &[String]) {
    let live = has(args, "--live");
    let fresh = has(args, "--fresh");
    let check = has(args, "--check");
    for arg in args {
        if arg.starts_with("--") && !["--live", "--fresh", "--check"].contains(&arg.as_str()) {
            usage();
        }
    }
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
    }
    let setup_key = setup_key_mint(&xtask);
    let cfg = load_runtime_config();
    let review_link = match staff_review_link(cfg.bind, cfg.bind6, ADMIN_PORT) {
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
    eprintln!("usage: cargo xtask up [--live] [--fresh] [--check]");
    std::process::exit(2);
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

/// Toolchain pin check: `rustc` must carry the `rust-toolchain.toml`
/// channel, `node` the `.nvmrc` major. A mismatch fails closed — a
/// build under the wrong toolchain is never "close enough".
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
        fail(&format!("node {out:?} does not match .nvmrc {want:?}"));
    }
    println!("up: toolchain ok (rust {channel}, node {want})");
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

/// Pure: `node --version` output (`v26.10.0`) satisfies the `.nvmrc`
/// pin (`26` or `26.10.0`).
fn node_matches(want: &str, node_version: &str) -> bool {
    let got = node_version.trim().trim_start_matches('v');
    got == want || got.starts_with(&format!("{want}."))
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
    run_cmd("npm", &["ci", "--prefix", "web"]);
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

/// Seal the loopback config: 127.0.0.1 binds, local ports, localhost
/// WebAuthn origin, the freshly built SPA/static directories, and the
/// local-dev admin seam. `admin.mtls_off` is what lets the admin host
/// guard accept loopback browsers; the release dev-seam gate only
/// honors it while every bind stays loopback.
fn config_set_all(xtask: &Path) {
    const SETTINGS: &[(&str, &str, &str)] = &[
        ("net.bind", "text", "127.0.0.1"),
        ("net.port", "text", "18318"),
        ("net.admin_port", "text", "18319"),
        ("auth.rp_id", "text", "localhost"),
        ("auth.origin", "text", "http://localhost:18318"),
        ("paths.static_dir", "text", "crates/web/static"),
        ("paths.spa_dir", "text", "web/build"),
        ("paths.admin_spa_dir", "text", "web/build-admin"),
        ("admin.mtls_off", "bool", "true"),
    ];
    for (key, kind, value) in SETTINGS {
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
        let authority = authority_id(&conn, slug);
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

fn authority_id(conn: &rusqlite::Connection, slug: &str) -> i64 {
    use rusqlite::OptionalExtension as _;
    conn.query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |row| {
        row.get(0)
    })
    .optional()
    .unwrap_or_else(|e| fail(&format!("authority lookup: {e}")))
    .unwrap_or_else(|| fail(&format!("packs did not create authority {slug}")))
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before 1970")
        .as_secs()
        .try_into()
        .expect("unix time exceeds i64")
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
/// listens past localhost.
fn staff_review_link(
    bind: std::net::IpAddr,
    bind6: Option<std::net::IpAddr>,
    admin_port: u16,
) -> Result<String, String> {
    if !sealed_config::RuntimeConfig::dev_seam_allowed(true, bind, bind6) {
        return Err("every bind IP must be loopback (refusing to mint a staff link)".to_string());
    }
    Ok(match bind {
        std::net::IpAddr::V4(_) => format!("http://{bind}:{admin_port}/review"),
        std::net::IpAddr::V6(_) => format!("http://[{bind}]:{admin_port}/review"),
    })
}

/// Release `web` environment: the store plus the file KEK in
/// `dev:`-form. The binary runs in release, so the dev-seam gate
/// checks the sealed binds — loopback here, so it boots.
fn web_env() -> Vec<(String, String)> {
    let raw = std::fs::read(KEK_FILE).unwrap_or_else(|e| fail(&format!("read {KEK_FILE}: {e}")));
    vec![
        ("STORE_DIR".to_string(), STORE_DIR.to_string()),
        ("HANDBOOK_DEV_KEK".to_string(), format!("dev:{}", hex(&raw))),
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
    if !wait_healthy() {
        failure = Some("web did not answer /healthz in time".to_string());
    }
    for (host, port, path, want) in [
        ("127.0.0.1", PUBLIC_PORT, "/apostille/healthz", "ok"),
        (
            "127.0.0.1",
            PUBLIC_PORT,
            "/apostille/api/v1/authorities",
            "authorities",
        ),
        (
            "127.0.0.1",
            ADMIN_PORT,
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

fn wait_healthy() -> bool {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        match http_get("127.0.0.1", PUBLIC_PORT, "/apostille/healthz") {
            Ok((200, _)) => return true,
            _ => std::thread::sleep(Duration::from_millis(500)),
        }
    }
    false
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn print_banner(setup_key: &str, review_link: &str) {
    println!();
    println!("handbook is up (loopback only, synthetic data):");
    println!("  public:  http://localhost:{PUBLIC_PORT}/apostille");
    println!("  admin:   http://localhost:{ADMIN_PORT}/");
    println!("  review:  {review_link}");
    println!("  setup key (prints ONCE — paste at /config to enrol staff):");
    println!("    {setup_key}");
    println!("try this:");
    println!("  - open the review queue and approve a proposed fact");
    println!("  - curl -s http://127.0.0.1:{PUBLIC_PORT}/apostille/api/v1/authorities");
    println!("  - POST {{\"sample\":\"syn-doc-clean\"}} to /apostille/api/v1/demo/precheck");
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
        assert!(node_matches("26", "v26.10.0\n"));
        assert!(node_matches("26.10.0", "v26.10.0"));
        assert!(!node_matches("26", "v24.1.0"));
        assert!(!node_matches("26", "v260.0.0"));
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
    fn staff_link_needs_loopback() {
        use std::net::{IpAddr, Ipv6Addr};
        let loopback = IpAddr::from([127, 0, 0, 1]);
        let lan: IpAddr = "192.168.1.2".parse().unwrap();
        assert_eq!(
            staff_review_link(loopback, None, 18319).as_deref(),
            Ok("http://127.0.0.1:18319/review")
        );
        assert_eq!(
            staff_review_link(loopback, Some(IpAddr::from(Ipv6Addr::LOCALHOST)), 18319).as_deref(),
            Ok("http://127.0.0.1:18319/review")
        );
        assert_eq!(
            staff_review_link(IpAddr::from(Ipv6Addr::LOCALHOST), None, 18319).as_deref(),
            Ok("http://[::1]:18319/review")
        );
        assert!(staff_review_link(lan, None, 18319).is_err());
        assert!(staff_review_link(loopback, Some(lan), 18319).is_err());
    }
}
