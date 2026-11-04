//! `cargo xtask setup wizard`: the setup file in a browser.
//!
//! A small server on `127.0.0.1` (random port). The printed URL carries
//! a token, swapped for an HttpOnly SameSite=Strict session cookie. The
//! link works again for [`GRACE_SECS`] after its first use (a second tab,
//! a reload); after that a reuse waits for the operator to confirm a
//! restart in the terminal, which mints a new link and ends every
//! session. Every request must name this exact host and port in
//! `Host` (anti-rebinding); every write must carry the page's CSRF
//! header and, when sent, a matching `Origin`. Secrets go in and never
//! come back: the page sees only "set". Each change is saved at once,
//! through sops (or, for the plain local-only `.local/setup.yaml`, as
//! 0600 YAML), so the file on disk is always the truth.
//!
//! The page edits targets and config values, runs the key checks,
//! probes a target's box over SSH, runs a target with its phases and
//! log streaming live (SSE), runs, stops or removes every unit at once
//! ([`batch`]; servers are never stopped or removed), and walks the
//! brand-licence step.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path as UrlPath, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{
        Html, IntoResponse, Redirect, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use super::batch::{self, All};
use super::brand;
use super::checks;
use super::file::{self, Runtime, SetupFile, TargetKind};
use super::map;
use super::plan::{self, Phase};
use super::run::Event;

const PAGE: &str = include_str!("wizard.html");
const COOKIE: &str = "hb_setup";
/// How long the printed link keeps working after its first use.
pub const GRACE_SECS: i64 = 120 * 60;
const REUSED: &str = "Link was reused outside grace period, please confirm on console that you want to restart the configuration process";

/// What runs where, said once in the terminal (the page says more).
const INTRO: &str = "  Local only: the handbook and staff console, drafting (with a meta.api_key), passkeys for this browser
  on localhost, and fakes for FedEx/USPS labels, Stripe, Telnyx and email (the local target's defaults).
  Needs a host on the internet: customer passkeys on your domain, Google and Apple sign-in, Stripe and
  Telnyx callbacks, real email delivery and TLS certificates.";

struct Inner {
    file: SetupFile,
    /// The link's token (a restart mints a new one).
    token: String,
    /// When the link was first used (unix seconds).
    token_used_at: Option<i64>,
    session: Option<String>,
    /// A late reuse is waiting for the terminal's answer.
    restart_pending: bool,
    running: bool,
    history: Vec<String>,
}

#[derive(Clone)]
struct App {
    inner: Arc<Mutex<Inner>>,
    path: PathBuf,
    repo: PathBuf,
    host: String,
    csrf: String,
    events: broadcast::Sender<String>,
    /// The local target's boot, serving in the background (one at a time).
    local: Arc<Mutex<Option<std::process::Child>>>,
    /// Open new links in the browser (`--no-open` turns it off).
    open_browser: bool,
}

fn random_hex() -> String {
    let bytes: [u8; 32] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn deny(code: StatusCode, message: &str) -> Response {
    (code, Json(json!({ "error": message }))).into_response()
}

/// A plain page for a link the browser opened (no session yet).
fn notice(code: StatusCode, message: &str) -> Response {
    let text = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let page = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Handbook setup</title>\
         <style>body{{font:15px/1.5 -apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif;margin:15vh auto;max-width:36rem;padding:0 16px}}</style>\
         </head><body><h1>Handbook setup</h1><p>{text}</p></body></html>"
    );
    (
        code,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::X_FRAME_OPTIONS, "DENY"),
        ],
        Html(page),
    )
        .into_response()
}

/// What a `?t=` link is worth now.
#[derive(Debug, PartialEq)]
enum Link {
    /// This run's token, unused or within the grace period.
    Good,
    /// This run's token, more than [`GRACE_SECS`] after its first use.
    Late,
    /// Not this run's token.
    Unknown,
}

fn link_state(inner: &Inner, token: &str, now: i64) -> Link {
    if token != inner.token {
        return Link::Unknown;
    }
    match inner.token_used_at {
        Some(first) if now - first > GRACE_SECS => Link::Late,
        _ => Link::Good,
    }
}

/// Mint a new link and end every session; returns the URL.
fn restart(app: &App) -> String {
    let mut inner = app.inner.lock().unwrap();
    inner.token = random_hex();
    inner.token_used_at = None;
    inner.session = None;
    inner.restart_pending = false;
    format!("http://{}/?t={}", app.host, inner.token)
}

fn open_url(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(url).status();
}

/// Ask in the terminal whether a late reuse may restart the wizard.
fn ask_restart(app: &App) {
    use std::io::{BufRead as _, IsTerminal as _, Write as _};
    let app = app.clone();
    std::thread::spawn(move || {
        println!(
            "\nsetup wizard: the link was opened again more than {} minutes after its first use.",
            GRACE_SECS / 60
        );
        if !std::io::stdin().is_terminal() {
            println!(
                "  No terminal to confirm in: stop the wizard (Ctrl-C) and start it again for a new link."
            );
            return;
        }
        print!(
            "  Restart the configuration process with a new link (signs out every open wizard tab)? [y/N] "
        );
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
            let url = restart(&app);
            println!(
                "setup wizard: {url}\n  (works for {} minutes after its first use)",
                GRACE_SECS / 60
            );
            if app.open_browser {
                open_url(&url);
            }
        } else {
            app.inner.lock().unwrap().restart_pending = false;
            println!("  Kept the current session; the old link stays refused.");
        }
    });
}

// Handlers return `Response` directly; the guards hand theirs back the
// same way, so the large error variant is the point, not an accident.
#[allow(clippy::result_large_err)]
impl App {
    fn host_ok(&self, headers: &HeaderMap) -> bool {
        headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| h == self.host)
    }

    fn session_ok(&self, headers: &HeaderMap) -> bool {
        let want = self.inner.lock().unwrap().session.clone();
        let Some(want) = want else { return false };
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(';'))
            .filter_map(|c| c.trim().strip_prefix(&format!("{COOKIE}=")))
            .any(|v| v == want)
    }

    /// Reads: right host and a session. Writes: also CSRF and Origin.
    fn guard(&self, headers: &HeaderMap, write: bool) -> Result<(), Response> {
        if !self.host_ok(headers) {
            return Err(deny(StatusCode::MISDIRECTED_REQUEST, "wrong Host"));
        }
        if !self.session_ok(headers) {
            return Err(deny(
                StatusCode::UNAUTHORIZED,
                "open the URL printed in the terminal",
            ));
        }
        if write {
            let csrf = headers.get("x-setup-csrf").and_then(|v| v.to_str().ok());
            if csrf != Some(self.csrf.as_str()) {
                return Err(deny(StatusCode::FORBIDDEN, "missing CSRF header"));
            }
            if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
                && origin != format!("http://{}", self.host)
            {
                return Err(deny(StatusCode::FORBIDDEN, "cross-origin write"));
            }
        }
        Ok(())
    }

    fn save(&self, inner: &Inner) -> Result<(), Response> {
        file::save(&self.path, &inner.file).map_err(|e| deny(StatusCode::INTERNAL_SERVER_ERROR, &e))
    }
}

#[derive(Deserialize)]
struct TokenQuery {
    t: Option<String>,
}

async fn index(
    State(app): State<App>,
    Query(q): Query<TokenQuery>,
    headers: HeaderMap,
) -> Response {
    if !app.host_ok(&headers) {
        return deny(StatusCode::MISDIRECTED_REQUEST, "wrong Host");
    }
    if let Some(t) = q.t {
        let now = crate::unix_now();
        let mut inner = app.inner.lock().unwrap();
        match link_state(&inner, &t, now) {
            Link::Good => {
                // Within the grace period every use joins one session.
                inner.token_used_at.get_or_insert(now);
                let session = inner.session.get_or_insert_with(random_hex).clone();
                let cookie = format!("{COOKIE}={session}; HttpOnly; SameSite=Strict; Path=/");
                return ([(header::SET_COOKIE, cookie)], Redirect::to("/")).into_response();
            }
            Link::Late => {
                if !inner.restart_pending {
                    inner.restart_pending = true;
                    drop(inner);
                    ask_restart(&app);
                }
                return notice(StatusCode::UNAUTHORIZED, REUSED);
            }
            Link::Unknown => {
                return notice(
                    StatusCode::UNAUTHORIZED,
                    "This link is not from the wizard that is running now. Open the URL printed in the terminal.",
                );
            }
        }
    }
    if !app.session_ok(&headers) {
        return deny(
            StatusCode::UNAUTHORIZED,
            "open the URL printed in the terminal",
        );
    }
    let page = PAGE.replace("__CSRF__", &app.csrf);
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, "default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' data:"),
            (header::X_FRAME_OPTIONS, "DENY"),
        ],
        Html(page),
    )
        .into_response()
}

/// A value as the page may see it: secrets only as set / unset.
fn shown(key: &str, value: Option<&Value>) -> Value {
    match value {
        None => Value::Null,
        Some(_) if file::is_secret(key) => json!({ "secret": true }),
        Some(v) => v.clone(),
    }
}

fn state_json(app: &App, inner: &Inner) -> Value {
    let f = &inner.file;
    let keys: Vec<Value> = sealed_config::keys::ALL
        .iter()
        .filter(|(k, _)| !k.starts_with("setup."))
        .map(|(k, t)| json!({ "key": k, "type": t.as_str(), "secret": file::is_secret(k) }))
        .collect();
    let targets: BTreeMap<&String, Value> = f
        .targets
        .iter()
        .map(|(name, t)| {
            let derived = file::derived(t);
            let own: BTreeMap<&String, Value> = t
                .config
                .keys()
                .map(|k| (k, shown(k, t.config.get(k))))
                .collect();
            (
                name,
                json!({
                    "kind": t.kind, "runtime": t.runtime,
                    "ssh": t.ssh, "ssh_admin": t.ssh_admin, "public_host": t.public_host,
                    "bind": t.bind, "bind6": t.bind6, "dns": t.dns, "timers": t.timers,
                    "platform": t.platform, "deploy_keys": t.deploy_keys,
                    "probe": t.probe,
                    "config": own,
                    "derived": derived,
                }),
            )
        })
        .collect();
    let shared: BTreeMap<&String, Value> = f
        .config
        .keys()
        .map(|k| (k, shown(k, f.config.get(k))))
        .collect();
    let dir = brand::dir(&app.repo, f);
    let assets: Vec<Value> = brand::ASSETS
        .iter()
        .map(|a| {
            let licence = f.brand.licences.iter().find(|l| l.asset == a.id);
            let files: Vec<Value> = a
                .files
                .iter()
                .map(|name| {
                    let bytes = std::fs::read(dir.join(name)).ok();
                    let sha = bytes.as_deref().map(brand::sha256_hex);
                    let accepted = licence.is_some_and(|l| l.files.iter().any(|x| Some(&x.sha256) == sha.as_ref() && x.name == *name));
                    json!({ "name": name, "present": sha.is_some(), "licensed": accepted,
                            "source": brand::source(f, name) })
                })
                .collect();
            json!({
                "id": a.id, "owner": a.owner, "terms": a.terms, "terms_url": a.terms_url,
                "permits": a.permits, "get": a.get, "files": files,
                "accepted": licence.map(|l| json!({ "by": l.accepted_by, "at": l.accepted_at, "version": l.terms_version })),
            })
        })
        .collect();
    json!({
        "path": app.path.display().to_string(),
        "operator": f.operator.name,
        "default_targets": f.default_targets,
        "keys": keys,
        "shared": shared,
        "targets": targets,
        "checks": checks::CHECKS.iter().map(|(id, label)| json!({ "id": id, "label": label })).collect::<Vec<_>>(),
        "phases": plan::ALL_PHASES.map(Phase::as_str),
        "brand": assets,
        "brand_dir": dir.display().to_string(),
        "problems": f.validate().err().unwrap_or_default(),
        "running": inner.running,
        "map": map::state(f),
        "all": {
            "run": batch::run_order(f),
            "stop": batch::describe(f, All::Stop),
            "remove": batch::describe(f, All::Remove),
            "servers": batch::servers(f),
            "order_needed": batch::run_order(f).iter().any(|n| n == "production"),
        },
    })
}

async fn api_state(State(app): State<App>, headers: HeaderMap) -> Response {
    if let Err(r) = app.guard(&headers, false) {
        return r;
    }
    let inner = app.inner.lock().unwrap();
    Json(state_json(&app, &inner)).into_response()
}

#[derive(Deserialize)]
struct SetValue {
    /// `shared` or a target name.
    scope: String,
    key: String,
    /// `null` deletes the value.
    value: Option<Value>,
}

async fn api_value(
    State(app): State<App>,
    headers: HeaderMap,
    Json(req): Json<SetValue>,
) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    let mut inner = app.inner.lock().unwrap();
    let value = req
        .value
        .filter(|v| !matches!(v, Value::String(s) if s.is_empty()));
    if let Some(v) = &value
        && let Err(e) = file::registry_value(&req.key, v)
    {
        return deny(StatusCode::BAD_REQUEST, &format!("{}: {e}", req.key));
    }
    let map = if req.scope == "shared" {
        if file::is_secret(&req.key) && value.is_some() {
            return deny(StatusCode::BAD_REQUEST, "secrets are set per target");
        }
        &mut inner.file.config
    } else if let Some(t) = inner.file.targets.get_mut(&req.scope) {
        &mut t.config
    } else {
        return deny(StatusCode::BAD_REQUEST, "no such target");
    };
    match value {
        Some(v) => map.insert(req.key, v),
        None => map.remove(&req.key),
    };
    if let Err(problems) = inner.file.validate() {
        // Keep the edit (the operator fixes the rest), but say why a run would refuse.
        let _ = app.save(&inner);
        return (
            StatusCode::OK,
            Json(json!({ "saved": true, "problems": problems })),
        )
            .into_response();
    }
    match app.save(&inner) {
        Ok(()) => Json(json!({ "saved": true })).into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize)]
struct SetField {
    target: String,
    field: String,
    value: Value,
}

async fn api_target(
    State(app): State<App>,
    headers: HeaderMap,
    Json(req): Json<SetField>,
) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    let mut inner = app.inner.lock().unwrap();
    let Some(t) = inner.file.targets.get_mut(&req.target) else {
        return deny(StatusCode::BAD_REQUEST, "no such target");
    };
    let text = req
        .value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let list = || -> Vec<String> {
        text.as_deref()
            .unwrap_or_default()
            .split([',', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    };
    match req.field.as_str() {
        "ssh" => t.ssh = text.clone(),
        "ssh_admin" => t.ssh_admin = text.clone(),
        "public_host" => t.public_host = text.clone(),
        "bind" => t.bind = text.clone(),
        "bind6" => t.bind6 = text.clone(),
        "platform" => t.platform = text.clone(),
        "dns" => t.dns = list(),
        "timers" => t.timers = list(),
        "deploy_keys" => {
            t.deploy_keys = text
                .as_deref()
                .unwrap_or_default()
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect()
        }
        _ => return deny(StatusCode::BAD_REQUEST, "unknown field"),
    }
    match app.save(&inner) {
        Ok(()) => Json(
            json!({ "saved": true, "problems": inner.file.validate().err().unwrap_or_default() }),
        )
        .into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize)]
struct Operator {
    name: String,
    #[serde(default)]
    default_targets: Option<Vec<String>>,
}

async fn api_operator(
    State(app): State<App>,
    headers: HeaderMap,
    Json(req): Json<Operator>,
) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    let mut inner = app.inner.lock().unwrap();
    inner.file.operator.name = req.name.trim().chars().take(120).collect();
    if let Some(targets) = req.default_targets {
        inner.file.default_targets = targets;
    }
    match app.save(&inner) {
        Ok(()) => Json(json!({ "saved": true })).into_response(),
        Err(r) => r,
    }
}

/// Only a target: no field can ask the wizard for a secret.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckReq {
    target: String,
}

async fn api_check(
    State(app): State<App>,
    headers: HeaderMap,
    Json(req): Json<CheckReq>,
) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    let f = app.inner.lock().unwrap().file.clone();
    match tokio::task::spawn_blocking(move || checks::run_target(&f, &req.target)).await {
        Ok(results) => Json(json!({ "results": results })).into_response(),
        Err(_) => deny(StatusCode::INTERNAL_SERVER_ERROR, "checks failed to run"),
    }
}

#[derive(Deserialize)]
struct ProbeReq {
    target: String,
    /// The ssh host or alias to probe (the page fills in the target's).
    host: String,
}

/// Probe a box over SSH (keys only) and cache what it is on the target.
async fn api_probe(
    State(app): State<App>,
    headers: HeaderMap,
    Json(req): Json<ProbeReq>,
) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    if !app
        .inner
        .lock()
        .unwrap()
        .file
        .targets
        .contains_key(&req.target)
    {
        return deny(StatusCode::BAD_REQUEST, "no such target");
    }
    let host = req.host.trim().to_string();
    if !super::probe::valid_host(&host) {
        return deny(
            StatusCode::BAD_REQUEST,
            "not a host name or ssh alias (letters, digits and .-_@: only)",
        );
    }
    let at = brand::rfc3339(crate::unix_now());
    let probe = match tokio::task::spawn_blocking(move || super::probe::run(&host, &at)).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return deny(StatusCode::BAD_GATEWAY, &e),
        Err(_) => return deny(StatusCode::INTERNAL_SERVER_ERROR, "the probe failed to run"),
    };
    let mut inner = app.inner.lock().unwrap();
    let Some(t) = inner.file.targets.get_mut(&req.target) else {
        return deny(StatusCode::BAD_REQUEST, "no such target");
    };
    t.probe = Some(probe.clone());
    match app.save(&inner) {
        Ok(()) => Json(json!({ "probe": probe })).into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize)]
struct MapReq {
    /// `front_end`, `workers` or `staging`, with `place`.
    #[serde(default)]
    node: Option<String>,
    /// `native`, `docker` or `server`.
    #[serde(default)]
    place: Option<String>,
    /// The Hugging Face repo the workers serve (`""` clears it).
    #[serde(default)]
    weights_repo: Option<String>,
}

/// The map's choices: where a node runs, and the weights repo.
async fn api_map(State(app): State<App>, headers: HeaderMap, Json(req): Json<MapReq>) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    let mut inner = app.inner.lock().unwrap();
    if let (Some(node), Some(place)) = (&req.node, &req.place)
        && let Err(e) = map::choose(&mut inner.file, node, place)
    {
        return deny(StatusCode::BAD_REQUEST, &e);
    }
    if let Some(repo) = req.weights_repo.as_deref().map(str::trim) {
        if !repo.is_empty() && !map::valid_repo(repo) {
            return deny(
                StatusCode::BAD_REQUEST,
                "a Hugging Face repo is org/name (letters, digits and ._-)",
            );
        }
        inner.file.map.weights_repo = (!repo.is_empty()).then(|| repo.to_string());
    }
    match app.save(&inner) {
        Ok(()) => Json(
            json!({ "saved": true, "problems": inner.file.validate().err().unwrap_or_default() }),
        )
        .into_response(),
        Err(r) => r,
    }
}

/// The map's Test button: `GET /v1/models` with the target's Meta key,
/// called from here so the key never reaches the page.
async fn api_meta(
    State(app): State<App>,
    headers: HeaderMap,
    Json(req): Json<CheckReq>,
) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    let f = app.inner.lock().unwrap().file.clone();
    match tokio::task::spawn_blocking(move || checks::meta_models(&f, &req.target)).await {
        Ok(Ok(models)) => Json(json!({ "models": models })).into_response(),
        Ok(Err(e)) => deny(StatusCode::BAD_GATEWAY, &e),
        Err(_) => deny(StatusCode::INTERNAL_SERVER_ERROR, "the test failed to run"),
    }
}

#[derive(Deserialize)]
struct RunReq {
    target: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    order: Option<String>,
    #[serde(default)]
    sha: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AllReq {
    /// `run`, `stop` or `remove`.
    action: String,
    /// Run all: the operator's order, required when production is in it.
    #[serde(default)]
    order: Option<String>,
    /// Remove all: the word `remove`, typed by the operator.
    #[serde(default)]
    confirm: Option<String>,
}

/// Run all, Stop all or Remove all ([`batch`]), streamed like a run.
async fn api_all(State(app): State<App>, headers: HeaderMap, Json(req): Json<AllReq>) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    let Some(action) = All::parse(&req.action) else {
        return deny(StatusCode::BAD_REQUEST, "action is run, stop or remove");
    };
    if action == All::Remove && req.confirm.as_deref() != Some("remove") {
        return deny(
            StatusCode::BAD_REQUEST,
            "type remove to confirm: it deletes local and staging data",
        );
    }
    let order = req
        .order
        .map(|o| o.trim().to_string())
        .filter(|o| !o.is_empty());
    let (f, targets) = {
        let mut inner = app.inner.lock().unwrap();
        if inner.running {
            return deny(StatusCode::CONFLICT, "a run is in progress");
        }
        let f = inner.file.clone();
        let targets = match action {
            All::Run => {
                if let Err(problems) = f.validate() {
                    return deny(StatusCode::BAD_REQUEST, &problems.join("; "));
                }
                let targets = batch::run_order(&f);
                if targets.iter().any(|n| n == "production") && order.is_none() {
                    return deny(
                        StatusCode::BAD_REQUEST,
                        "Run all includes production: fill in the operator order (who, when, what)",
                    );
                }
                targets
            }
            All::Stop | All::Remove => batch::local_targets(&f),
        };
        inner.running = true;
        inner.history.clear();
        (f, targets)
    };
    emit_event(
        &app,
        &Event::Batch {
            action: action.as_str().to_string(),
            targets: targets.clone(),
        },
    );
    let (worker, list) = (app.clone(), targets.clone());
    std::thread::spawn(move || run_batch(&worker, &f, action, &list, order.as_deref()));
    (
        StatusCode::ACCEPTED,
        Json(json!({ "started": true, "targets": targets })),
    )
        .into_response()
}

/// The batch itself, on its own thread. Run all stops at the first
/// failed target; Stop and Remove go on and report every failure.
fn run_batch(app: &App, f: &SetupFile, action: All, targets: &[String], order: Option<&str>) {
    let emit = |event: Event| emit_event(app, &event);
    let done = |ok: bool, detail: String| {
        emit(Event::BatchDone { ok, detail });
        app.inner.lock().unwrap().running = false;
    };
    let mut failed = Vec::new();
    for name in targets {
        let local_boot = f
            .targets
            .get(name)
            .is_some_and(|t| t.kind == TargetKind::Local && t.runtime == Runtime::Native);
        let result = match action {
            All::Run if local_boot => {
                // Last in the order: it keeps serving, and `start_local`
                // clears `running` once its banner prints.
                if app.local.lock().unwrap().is_some() {
                    Err(format!("{name} is already serving"))
                } else {
                    emit(Event::BatchDone {
                        ok: true,
                        detail: format!("booting {name} last (it keeps serving)"),
                    });
                    match start_local(app, name) {
                        Ok(()) => return,
                        Err(e) => Err(e),
                    }
                }
            }
            All::Run => super::run_one(f, name, "HEAD", None, order, &emit),
            All::Stop | All::Remove => batch::steps(f, name, action).and_then(|steps| {
                let ctx = super::run::Ctx {
                    repo: app.repo.clone(),
                    file: f,
                    target: name.clone(),
                    facts: super::plan::Facts {
                        sha: action.as_str().to_string(),
                        arch: super::build::Arch::X86_64,
                        migration_ack: None,
                    },
                    emit: &emit,
                };
                super::run::execute(&ctx, &steps, None)
            }),
        };
        if let Err(e) = result {
            if action == All::Run {
                return done(false, format!("Run all stopped at {name}: {e}"));
            }
            failed.push(format!("{name}: {e}"));
        }
    }
    let left = batch::servers(f);
    let left = if action != All::Run && !left.is_empty() {
        format!(
            "; {} left alone (servers are never stopped or removed here)",
            left.join(", ")
        )
    } else {
        String::new()
    };
    if failed.is_empty() {
        done(true, format!("{} all: done{left}", action.as_str()));
    } else {
        done(
            false,
            format!("{} all: {}{left}", action.as_str(), failed.join("; ")),
        );
    }
}

async fn api_run(State(app): State<App>, headers: HeaderMap, Json(req): Json<RunReq>) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    let f = {
        let mut inner = app.inner.lock().unwrap();
        if inner.running {
            return deny(StatusCode::CONFLICT, "a run is in progress");
        }
        if let Err(problems) = inner.file.validate() {
            return deny(StatusCode::BAD_REQUEST, &problems.join("; "));
        }
        // A native local boot serves in the foreground (a child below);
        // in Docker it is a plain run that starts the judge image.
        let local = inner
            .file
            .targets
            .get(&req.target)
            .is_some_and(|t| t.kind == TargetKind::Local && t.runtime == Runtime::Native);
        if local && app.local.lock().unwrap().is_some() {
            return deny(
                StatusCode::CONFLICT,
                "the local target is already serving; stop the wizard (Ctrl-C) to stop it",
            );
        }
        inner.running = true;
        inner.history.clear();
        if local {
            drop(inner);
            return match start_local(&app, &req.target) {
                Ok(()) => (StatusCode::ACCEPTED, Json(json!({ "started": true }))).into_response(),
                Err(e) => {
                    app.inner.lock().unwrap().running = false;
                    deny(StatusCode::INTERNAL_SERVER_ERROR, &e)
                }
            };
        }
        inner.file.clone()
    };
    let from = match req.from.as_deref().filter(|s| !s.is_empty()) {
        Some(p) => match Phase::parse(p) {
            Some(phase) => Some(phase),
            None => return deny(StatusCode::BAD_REQUEST, "unknown phase"),
        },
        None => None,
    };
    let worker = app.clone();
    std::thread::spawn(move || {
        let emit = |event: Event| {
            let line = serde_json::to_string(&event).unwrap_or_default();
            worker.inner.lock().unwrap().history.push(line.clone());
            let _ = worker.events.send(line);
        };
        let rev = req.sha.unwrap_or_else(|| "HEAD".into());
        if let Err(e) = super::run_one(&f, &req.target, &rev, from, req.order.as_deref(), &emit) {
            emit(Event::Finished {
                target: req.target.clone(),
                ok: false,
                detail: e,
            });
        }
        worker.inner.lock().unwrap().running = false;
    });
    (StatusCode::ACCEPTED, Json(json!({ "started": true }))).into_response()
}

/// Record and broadcast one run event (what the SSE stream replays).
fn emit_event(app: &App, event: &Event) {
    let line = serde_json::to_string(event).unwrap_or_default();
    app.inner.lock().unwrap().history.push(line.clone());
    let _ = app.events.send(line);
}

/// Free ports for a local boot, saved on the target when they moved so
/// the map's links and Stop all follow the server. See
/// [`crate::up::local_ports`].
fn settle_local_ports(app: &App, name: &str) -> Result<((u16, u16), Option<String>), String> {
    let mut inner = app.inner.lock().unwrap();
    let target = inner
        .file
        .targets
        .get_mut(name)
        .ok_or_else(|| format!("no target {name:?}"))?;
    let want = target.ports_or_default();
    let (pair, note) = crate::up::local_ports((want.public, want.admin), &app.repo)?;
    if note.is_some() {
        target.ports = Some(file::Ports {
            public: pair.0,
            admin: pair.1,
        });
        file::save(&app.path, &inner.file)?;
    }
    Ok((pair, note))
}

/// The local target serves in the foreground, so the wizard runs it as
/// a child (`xtask setup --target <name> --file <path>`, the same boot
/// as in a terminal) and streams its output into the run log. The run
/// counts as finished once `up`'s banner (setup key included) has
/// printed; the child keeps serving, its later output goes to this
/// terminal, and Ctrl-C on the wizard stops it with its server and
/// worker (one process group). One local boot at a time: killing just
/// the child would orphan its server on the ports.
fn start_local(app: &App, name: &str) -> Result<(), String> {
    use std::io::BufRead as _;
    use std::process::{Command, Stdio};
    let ((public, admin), moved) = settle_local_ports(app, name)?;
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let mut child = Command::new(exe)
        .args(["setup", "--target", name, "--file"])
        .arg(&app.path)
        .current_dir(&app.repo)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("start the local boot: {e}"))?;
    let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
    *app.local.lock().unwrap() = Some(child);
    emit_event(
        app,
        &Event::Plan {
            target: name.to_string(),
            version: "this checkout".to_string(),
            steps: vec![[
                Phase::Boot.as_str().to_string(),
                "cargo xtask setup --target local (keeps serving in the background)".to_string(),
            ]],
        },
    );
    emit_event(
        app,
        &Event::Start {
            index: 0,
            phase: Phase::Boot.as_str().to_string(),
            title: "boot the handbook on loopback with this file's values".to_string(),
        },
    );
    if let Some(note) = moved {
        emit_event(
            app,
            &Event::Line {
                index: 0,
                text: format!("{note} (saved on this target)"),
            },
        );
    }
    // Set once the banner has printed: later lines are server logs.
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let target = name.to_string();
    let pump = |pipe: Box<dyn std::io::Read + Send>, watch_banner: bool| {
        let (app, finished, target) = (app.clone(), finished.clone(), target.clone());
        std::thread::spawn(move || {
            let mut in_banner = false;
            for line in std::io::BufReader::new(pipe).lines().map_while(Result::ok) {
                if finished.load(std::sync::atomic::Ordering::SeqCst) {
                    println!("[{target}] {line}");
                    continue;
                }
                in_banner |= watch_banner && line.starts_with("handbook is up");
                let banner_done = in_banner && line.trim().is_empty();
                emit_event(
                    &app,
                    &Event::Line {
                        index: 0,
                        text: line,
                    },
                );
                if banner_done && !finished.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    emit_event(
                        &app,
                        &Event::Done {
                            index: 0,
                            ok: true,
                            detail: String::new(),
                        },
                    );
                    emit_event(
                        &app,
                        &Event::Finished {
                            target: target.clone(),
                            ok: true,
                            detail: format!(
                                "{target} is up: public http://localhost:{public}/apostille, staff http://localhost:{admin}/config (paste the setup key printed above; a reboot prints a new one until a passkey is enrolled). Its log continues in the wizard's terminal; Stop all or Ctrl-C there stops it."
                            ),
                        },
                    );
                    app.inner.lock().unwrap().running = false;
                }
            }
        });
    };
    if let Some(out) = stdout {
        pump(Box::new(out), true);
    }
    if let Some(err) = stderr {
        pump(Box::new(err), false);
    }
    // Reap the child; report it if it stops before the banner.
    let (app, target) = (app.clone(), name.to_string());
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let mut slot = app.local.lock().unwrap();
            let Some(child) = slot.as_mut() else { return };
            let Ok(Some(status)) = child.try_wait() else {
                continue;
            };
            slot.take();
            drop(slot);
            if !finished.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let detail = format!("the local boot exited ({status})");
                emit_event(
                    &app,
                    &Event::Done {
                        index: 0,
                        ok: false,
                        detail: detail.clone(),
                    },
                );
                emit_event(
                    &app,
                    &Event::Finished {
                        target,
                        ok: false,
                        detail,
                    },
                );
                app.inner.lock().unwrap().running = false;
            } else {
                println!("[{target}] exited ({status})");
            }
            return;
        }
    });
    Ok(())
}

async fn api_events(State(app): State<App>, headers: HeaderMap) -> Response {
    if let Err(r) = app.guard(&headers, false) {
        return r;
    }
    let history = app.inner.lock().unwrap().history.clone();
    let rx = app.events.subscribe();
    let backlog = futures_util::stream::iter(
        history
            .into_iter()
            .map(|l| Ok::<_, std::convert::Infallible>(SseEvent::default().data(l))),
    );
    let live = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(line) => return Some((Ok(SseEvent::default().data(line)), rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    use futures_util::StreamExt as _;
    Sse::new(backlog.chain(live))
        .keep_alive(KeepAlive::default())
        .into_response()
}

#[derive(Deserialize)]
struct Accept {
    asset: String,
    terms_version: String,
    /// The operator ticked "I have read and accept".
    accept: bool,
}

async fn api_licence(
    State(app): State<App>,
    headers: HeaderMap,
    Json(req): Json<Accept>,
) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    let mut inner = app.inner.lock().unwrap();
    if !req.accept {
        inner.file.brand.licences.retain(|l| l.asset != req.asset);
    } else if let Err(e) = brand::accept(
        &app.repo,
        &mut inner.file,
        &req.asset,
        req.terms_version.trim(),
        &brand::rfc3339(crate::unix_now()),
    ) {
        return deny(StatusCode::BAD_REQUEST, &e);
    }
    match app.save(&inner) {
        Ok(()) => Json(json!({ "saved": true })).into_response(),
        Err(r) => r,
    }
}

async fn api_brand_file(
    State(app): State<App>,
    UrlPath(name): UrlPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    let Some(name) = brand::allowed_file(&name) else {
        return deny(
            StatusCode::BAD_REQUEST,
            "not a brand file name this build uses",
        );
    };
    // Same checks as a fetched file: size, type, and no SVG that runs
    // or loads anything (a dropped file used to skip the SVG check).
    if let Err(e) = brand::check_content(name, &body) {
        return deny(StatusCode::BAD_REQUEST, &e);
    }
    let dir = {
        let inner = app.inner.lock().unwrap();
        brand::dir(&app.repo, &inner.file)
    };
    if let Err(e) =
        std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(dir.join(name), &body))
    {
        return deny(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    Json(json!({ "saved": name, "sha256": brand::sha256_hex(&body) })).into_response()
}

#[derive(Deserialize)]
struct BrandFetch {
    file: String,
    /// The source as typed (kept in the setup file; empty: the default).
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    pick: Option<String>,
}

/// Fetch one brand file from its source, after the operator accepted
/// its owner's terms, and record its hash in that acceptance.
async fn api_brand_fetch(
    State(app): State<App>,
    headers: HeaderMap,
    Json(req): Json<BrandFetch>,
) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    let Some(name) = brand::allowed_file(&req.file) else {
        return deny(
            StatusCode::BAD_REQUEST,
            "not a brand file name this build uses",
        );
    };
    let f = {
        let mut inner = app.inner.lock().unwrap();
        if let Some(url) = req.url.as_deref().map(str::trim) {
            if !url.is_empty() && (!url.starts_with("https://") || url.len() > 2000) {
                return deny(
                    StatusCode::BAD_REQUEST,
                    "the source must be an https:// URL",
                );
            }
            let typed = file::BrandSource {
                url: url.to_string(),
                pick: req
                    .pick
                    .as_deref()
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(|p| p.chars().take(300).collect()),
            };
            let sources = &mut inner.file.brand.sources;
            if url.is_empty() || brand::default_source(name).as_ref() == Some(&typed) {
                sources.remove(name);
            } else {
                sources.insert(name.to_string(), typed);
            }
            if let Err(r) = app.save(&inner) {
                return r;
            }
        }
        inner.file.clone()
    };
    let repo = app.repo.clone();
    let fetched =
        tokio::task::spawn_blocking(move || brand::fetch_file(&repo, &f, name, &brand::download))
            .await;
    let sha = match fetched {
        Ok(Ok(sha)) => sha,
        Ok(Err(e)) => return deny(StatusCode::BAD_GATEWAY, &e),
        Err(_) => return deny(StatusCode::INTERNAL_SERVER_ERROR, "the fetch failed to run"),
    };
    let mut inner = app.inner.lock().unwrap();
    if let Err(e) = brand::record(&mut inner.file, name, &sha) {
        return deny(StatusCode::CONFLICT, &e);
    }
    match app.save(&inner) {
        Ok(()) => Json(json!({ "saved": name, "sha256": sha })).into_response(),
        Err(r) => r,
    }
}

/// Fill a local target's missing [`file::LOCAL_DEFAULTS`].
async fn api_defaults(
    State(app): State<App>,
    headers: HeaderMap,
    Json(req): Json<CheckReq>,
) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    let mut inner = app.inner.lock().unwrap();
    let Some(t) = inner.file.targets.get_mut(&req.target) else {
        return deny(StatusCode::BAD_REQUEST, "no such target");
    };
    if t.kind != TargetKind::Local {
        return deny(
            StatusCode::BAD_REQUEST,
            "local-only defaults (fakes) are for local targets",
        );
    }
    let added = file::fill_local_defaults(t);
    match app.save(&inner) {
        Ok(()) => Json(json!({ "added": added })).into_response(),
        Err(r) => r,
    }
}

/// Pull a server target's plain values beside the setup file and say
/// where they differ from it (secrets by presence only; reading their
/// values is `setup pull --secrets`, CLI only).
async fn api_pull(
    State(app): State<App>,
    headers: HeaderMap,
    Json(req): Json<CheckReq>,
) -> Response {
    if let Err(r) = app.guard(&headers, true) {
        return r;
    }
    let f = app.inner.lock().unwrap().file.clone();
    let path = app.path.clone();
    let at = brand::rfc3339(crate::unix_now());
    // Never `Some(reason)`: secrets are read from the CLI only, so no
    // browser tab ever holds one.
    match tokio::task::spawn_blocking(move || super::pull::run(&f, &path, &req.target, &at, None))
        .await
    {
        Ok(Ok((out, pulled, diffs))) => Json(json!({
            "path": out.display().to_string(),
            "values": pulled.values.len(),
            "secrets": pulled.secrets_set.len(),
            "diffs": diffs,
        }))
        .into_response(),
        Ok(Err(e)) => deny(StatusCode::BAD_GATEWAY, &e),
        Err(_) => deny(StatusCode::INTERNAL_SERVER_ERROR, "the pull failed to run"),
    }
}

fn router(app: App) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/state", get(api_state))
        .route("/api/value", post(api_value))
        .route("/api/target", post(api_target))
        .route("/api/operator", post(api_operator))
        .route("/api/check", post(api_check))
        .route("/api/probe", post(api_probe))
        .route("/api/map", post(api_map))
        .route("/api/meta", post(api_meta))
        .route("/api/defaults", post(api_defaults))
        .route("/api/pull", post(api_pull))
        .route("/api/run", post(api_run))
        .route("/api/all", post(api_all))
        .route("/api/events", get(api_events))
        .route("/api/licence", post(api_licence))
        .route("/api/brand/{name}", post(api_brand_file))
        .route("/api/brand-fetch", post(api_brand_fetch))
        .layer(axum::extract::DefaultBodyLimit::max(3 * 1024 * 1024))
        .with_state(app)
}

pub fn run(args: &[String], path: &Path) {
    let f = match super::load_or_none(path) {
        Some(f) => f,
        None => {
            let f = super::new_file(path, args);
            file::save(path, &f).unwrap_or_else(|e| super::fail(&e));
            println!(
                "setup: created {} ({})",
                path.display(),
                super::storage_note(path)
            );
            f
        }
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| super::fail(&e.to_string()));
    runtime.block_on(async move {
        let port: u16 = crate::flag(args, "port").and_then(|p| p.parse().ok()).unwrap_or(0);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap_or_else(|e| super::fail(&format!("bind 127.0.0.1: {e}")));
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
        let token = random_hex();
        let (events, _) = broadcast::channel(1024);
        let app = App {
            inner: Arc::new(Mutex::new(Inner {
                file: f,
                token: token.clone(),
                token_used_at: None,
                session: None,
                restart_pending: false,
                running: false,
                history: Vec::new(),
            })),
            path: path.to_path_buf(),
            repo: crate::repo_root(),
            host: format!("127.0.0.1:{port}"),
            csrf: random_hex(),
            events,
            local: Arc::new(Mutex::new(None)),
            open_browser: !args.iter().any(|a| a == "--no-open"),
        };
        let url = format!("http://127.0.0.1:{port}/?t={token}");
        println!(
            "setup wizard: {url}\n  (the link works for {} minutes after its first use; Ctrl-C stops the wizard; the file is saved on every change)\n\n{INTRO}\n",
            GRACE_SECS / 60
        );
        if app.open_browser {
            open_url(&url);
        }
        axum::serve(listener, router(app))
            .await
            .unwrap_or_else(|e| super::fail(&e.to_string()));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt as _;

    fn app() -> App {
        let (events, _) = broadcast::channel(8);
        let mut file = SetupFile::template();
        file.targets.get_mut("local").unwrap().config.insert(
            "meta.api_key".into(),
            Value::String("LLM_1_synthetic_never_shown".into()),
        );
        App {
            inner: Arc::new(Mutex::new(Inner {
                file,
                token: "tok".into(),
                token_used_at: None,
                session: Some("sess".into()),
                restart_pending: false,
                running: false,
                history: Vec::new(),
            })),
            path: PathBuf::from("/nonexistent/config.yaml"),
            repo: std::env::temp_dir(),
            host: "127.0.0.1:4000".into(),
            csrf: "csrf".into(),
            events,
            local: Arc::new(Mutex::new(None)),
            open_browser: false,
        }
    }

    async fn send(app: &App, req: axum::http::Request<axum::body::Body>) -> (StatusCode, String) {
        let res = router(app.clone()).oneshot(req).await.unwrap();
        let status = res.status();
        let body = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&body).to_string())
    }

    fn req(
        method: &str,
        uri: &str,
        host: &str,
        cookie: bool,
        csrf: bool,
    ) -> axum::http::Request<axum::body::Body> {
        let mut b = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header("host", host);
        if cookie {
            b = b.header("cookie", "hb_setup=sess");
        }
        if csrf {
            b = b.header("x-setup-csrf", "csrf");
        }
        b.header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"scope":"shared","key":"net.port","value":"9"}"#,
            ))
            .unwrap()
    }

    #[tokio::test]
    async fn guards_refuse_bad_host_missing_session_and_missing_csrf() {
        let app = app();
        assert_eq!(
            send(&app, req("GET", "/api/state", "evil.test", true, false))
                .await
                .0,
            StatusCode::MISDIRECTED_REQUEST
        );
        assert_eq!(
            send(
                &app,
                req("GET", "/api/state", "127.0.0.1:4000", false, false)
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            send(
                &app,
                req("POST", "/api/value", "127.0.0.1:4000", true, false)
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        let (status, body) = send(
            &app,
            req("GET", "/api/state", "127.0.0.1:4000", true, false),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            !body.contains("LLM_1_synthetic"),
            "a secret came back to the page"
        );
        assert!(body.contains(r#""meta.api_key":{"secret":true}"#), "{body}");
    }

    async fn open_link(app: &App, uri: &str) -> (StatusCode, Option<String>, String) {
        let res = router(app.clone())
            .oneshot(req("GET", uri, "127.0.0.1:4000", false, false))
            .await
            .unwrap();
        let status = res.status();
        let cookie = res
            .headers()
            .get("set-cookie")
            .map(|c| c.to_str().unwrap().to_string());
        let body = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, cookie, String::from_utf8_lossy(&body).to_string())
    }

    #[tokio::test]
    async fn the_link_works_through_its_grace_period_then_asks_the_terminal() {
        let app = app();
        app.inner.lock().unwrap().session = None;
        let (status, cookie, _) = open_link(&app, "/?t=tok").await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let cookie = cookie.unwrap();
        assert!(
            cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"),
            "{cookie}"
        );
        // A second tab within the grace period joins the same session.
        let (status, again, _) = open_link(&app, "/?t=tok").await;
        assert_eq!(status, StatusCode::SEE_OTHER, "reuse within grace");
        assert_eq!(again.as_deref(), Some(cookie.as_str()));
        // Past the grace period: the page says to confirm in the terminal.
        app.inner.lock().unwrap().token_used_at = Some(crate::unix_now() - GRACE_SECS - 1);
        let (status, cookie, body) = open_link(&app, "/?t=tok").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(cookie.is_none());
        assert!(body.contains(REUSED), "{body}");
        assert!(body.starts_with("<!doctype html>"), "a page, not JSON");
        assert!(app.inner.lock().unwrap().restart_pending);
        // A token from another run is refused outright.
        let (status, _, body) = open_link(&app, "/?t=other").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("not from the wizard"), "{body}");
        // The terminal's yes: a new link, and the old session ends.
        let url = restart(&app);
        let inner = app.inner.lock().unwrap();
        assert!(url.ends_with(&inner.token) && inner.token != "tok");
        assert!(inner.session.is_none() && inner.token_used_at.is_none());
        assert!(!inner.restart_pending);
    }

    #[tokio::test]
    async fn the_wizard_pull_takes_no_secrets_request() {
        let app = app();
        let (status, _) = send(
            &app,
            post(
                "/api/pull",
                r#"{"target":"production","secrets":true,"reason":"BDR drill"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn all_units_refuse_without_their_confirmations_and_never_list_a_server() {
        let app = app();
        for (body, want) in [
            (r#"{"action":"wipe"}"#, StatusCode::BAD_REQUEST),
            (r#"{"action":"remove"}"#, StatusCode::BAD_REQUEST),
            (
                r#"{"action":"remove","confirm":"yes"}"#,
                StatusCode::BAD_REQUEST,
            ),
            (r#"{"action":"run"}"#, StatusCode::BAD_REQUEST),
            (r#"{"action":"run","order":"   "}"#, StatusCode::BAD_REQUEST),
            (
                r#"{"action":"stop","target":"production"}"#,
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
        ] {
            let (status, text) = send(&app, post("/api/all", body)).await;
            assert_eq!(status, want, "{body}: {text}");
        }
        assert!(!app.inner.lock().unwrap().running, "nothing started");
        // One batch at a time, like one run.
        app.inner.lock().unwrap().running = true;
        let (status, _) = send(&app, post("/api/all", r#"{"action":"stop"}"#)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (_, state) = send(
            &app,
            req("GET", "/api/state", "127.0.0.1:4000", true, false),
        )
        .await;
        let state: Value = serde_json::from_str(&state).unwrap();
        let all = &state["all"];
        assert_eq!(all["run"], json!(["staging", "production"]));
        assert_eq!(all["servers"], json!(["production"]));
        assert_eq!(all["order_needed"], true);
        for list in ["stop", "remove"] {
            let lines = all[list].as_array().unwrap();
            assert_eq!(lines.len(), 3, "{list}: {lines:?}");
            assert!(
                lines
                    .iter()
                    .all(|l| !l.as_str().unwrap().starts_with("production"))
            );
        }
    }

    #[tokio::test]
    async fn local_defaults_fill_local_targets_only() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app();
        app.path = tmp.path().join("setup.yaml");
        let (status, text) = send(&app, post("/api/defaults", r#"{"target":"staging"}"#)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
        app.inner
            .lock()
            .unwrap()
            .file
            .targets
            .get_mut("local")
            .unwrap()
            .config
            .remove("labels.mode");
        let (status, text) = send(&app, post("/api/defaults", r#"{"target":"local"}"#)).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert_eq!(text, r#"{"added":["labels.mode"]}"#);
    }

    fn post(uri: &str, body: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method("POST")
            .uri(uri)
            .header("host", "127.0.0.1:4000")
            .header("cookie", "hb_setup=sess")
            .header("x-setup-csrf", "csrf")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn the_probe_refuses_option_shaped_hosts_and_unknown_targets() {
        let app = app();
        let (status, body) = send(
            &app,
            post(
                "/api/probe",
                r#"{"target":"staging","host":"-oProxyCommand=sh"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("not a host name"), "{body}");
        let (status, _) = send(
            &app,
            post("/api/probe", r#"{"target":"nope","host":"plant"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn the_map_refuses_unknown_places_and_repos_and_tests_meta_without_leaking() {
        let app = app();
        for body in [
            r#"{"node":"front_end","place":"cloud"}"#,
            r#"{"node":"cellar","place":"server"}"#,
            r#"{"weights_repo":"../etc"}"#,
        ] {
            let (status, text) = send(&app, post("/api/map", body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {text}");
        }
        let (status, text) = send(&app, post("/api/meta", r#"{"target":"staging"}"#)).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(text.contains("meta.api_key is not set"), "{text}");
        let (_, state) = send(
            &app,
            req("GET", "/api/state", "127.0.0.1:4000", true, false),
        )
        .await;
        assert!(state.contains(r#""front_end""#), "{state}");
        assert!(
            !state.contains("LLM_1_synthetic"),
            "a secret reached the map"
        );
    }

    /// A dropped file meets the same bar as a fetched one: an SVG that
    /// runs or loads anything never lands in the brand dir.
    #[tokio::test]
    async fn brand_uploads_meet_the_fetch_bar() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app();
        app.repo = tmp.path().to_path_buf();
        let saved = {
            let inner = app.inner.lock().unwrap();
            brand::dir(&app.repo, &inner.file).join("google.svg")
        };
        for bad in [
            r#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" onload="x()"></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="https://evil.test/a.png"/></svg>"#,
            "not an svg at all",
        ] {
            let (status, text) = send(&app, post("/api/brand/google.svg", bad)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {text}");
            assert!(!saved.exists(), "{bad} was written");
        }
        let clean = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><path d="M0 0h1v1z" fill="#4285F4"/></svg>"##;
        let (status, text) = send(&app, post("/api/brand/google.svg", clean)).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), clean);
    }

    #[tokio::test]
    async fn brand_fetch_waits_for_acceptance_and_takes_https_only() {
        let app = app();
        let (status, text) = send(&app, post("/api/brand-fetch", r#"{"file":"google.svg"}"#)).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(text.contains("tick acceptance of Google"), "{text}");
        for body in [
            r#"{"file":"../config.yaml"}"#,
            r#"{"file":"google.svg","url":"http://example.test/g.svg"}"#,
        ] {
            let (status, text) = send(&app, post("/api/brand-fetch", body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {text}");
        }
        let (_, state) = send(
            &app,
            req("GET", "/api/state", "127.0.0.1:4000", true, false),
        )
        .await;
        assert!(
            state.contains("signin-assets.zip"),
            "the default source is offered"
        );
    }
}
