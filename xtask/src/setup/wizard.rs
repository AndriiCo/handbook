//! `cargo xtask setup wizard`: the setup file in a browser.
//!
//! A small server on `127.0.0.1` (random port). The printed URL carries
//! a one-time token, swapped on first use for an HttpOnly SameSite=Strict
//! session cookie. Every request must name this exact host and port in
//! `Host` (anti-rebinding); every write must carry the page's CSRF
//! header and, when sent, a matching `Origin`. Secrets go in and never
//! come back: the page sees only "set". Each change is saved at once,
//! through sops (or, for the plain local-only `.local/setup.yaml`, as
//! 0600 YAML), so the file on disk is always the truth.
//!
//! The page edits targets and config values, runs the key checks,
//! probes a target's box over SSH, runs a target with its phases and
//! log streaming live (SSE), and walks the brand-licence step.

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

use super::brand;
use super::checks;
use super::file::{self, SetupFile, TargetKind};
use super::plan::{self, Phase};
use super::run::Event;

const PAGE: &str = include_str!("wizard.html");
const COOKIE: &str = "hb_setup";

struct Inner {
    file: SetupFile,
    token: Option<String>,
    session: Option<String>,
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
}

fn random_hex() -> String {
    let bytes: [u8; 32] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn deny(code: StatusCode, message: &str) -> Response {
    (code, Json(json!({ "error": message }))).into_response()
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
        let mut inner = app.inner.lock().unwrap();
        if inner.token.as_deref() == Some(t.as_str()) {
            // One use: the token dies, the cookie carries the session.
            inner.token = None;
            let session = random_hex();
            inner.session = Some(session.clone());
            let cookie = format!("{COOKIE}={session}; HttpOnly; SameSite=Strict; Path=/");
            return ([(header::SET_COOKIE, cookie)], Redirect::to("/")).into_response();
        }
        return deny(
            StatusCode::UNAUTHORIZED,
            "this link was already used; restart the wizard",
        );
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
                    "kind": t.kind,
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
                    json!({ "name": name, "present": sha.is_some(), "licensed": accepted })
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

#[derive(Deserialize)]
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
struct RunReq {
    target: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    order: Option<String>,
    #[serde(default)]
    sha: Option<String>,
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
        let local = inner.file.targets.get(&req.target).map(|t| t.kind) == Some(TargetKind::Local);
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
                                "{target} is up; its log continues in the wizard's terminal (Ctrl-C there stops it)"
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
    if body.is_empty() || body.len() > 2 * 1024 * 1024 {
        return deny(StatusCode::BAD_REQUEST, "file must be 1 byte to 2 MB");
    }
    let looks_right = if name.ends_with(".svg") {
        std::str::from_utf8(&body).is_ok_and(|s| s.contains("<svg"))
    } else {
        body.starts_with(b"wOF2")
    };
    if !looks_right {
        return deny(
            StatusCode::BAD_REQUEST,
            "file content does not match its type",
        );
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

fn router(app: App) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/state", get(api_state))
        .route("/api/value", post(api_value))
        .route("/api/target", post(api_target))
        .route("/api/operator", post(api_operator))
        .route("/api/check", post(api_check))
        .route("/api/probe", post(api_probe))
        .route("/api/run", post(api_run))
        .route("/api/events", get(api_events))
        .route("/api/licence", post(api_licence))
        .route("/api/brand/{name}", post(api_brand_file))
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
                token: Some(token.clone()),
                session: None,
                running: false,
                history: Vec::new(),
            })),
            path: path.to_path_buf(),
            repo: crate::repo_root(),
            host: format!("127.0.0.1:{port}"),
            csrf: random_hex(),
            events,
            local: Arc::new(Mutex::new(None)),
        };
        let url = format!("http://127.0.0.1:{port}/?t={token}");
        println!("setup wizard: {url}\n  (one-time link; Ctrl-C stops the wizard; the file is saved on every change)");
        if !args.iter().any(|a| a == "--no-open") {
            let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
            let _ = std::process::Command::new(opener).arg(&url).status();
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
                token: Some("tok".into()),
                session: Some("sess".into()),
                running: false,
                history: Vec::new(),
            })),
            path: PathBuf::from("/nonexistent/config.yaml"),
            repo: std::env::temp_dir(),
            host: "127.0.0.1:4000".into(),
            csrf: "csrf".into(),
            events,
            local: Arc::new(Mutex::new(None)),
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

    #[tokio::test]
    async fn the_token_works_once_and_sets_a_strict_cookie() {
        let app = app();
        let res = router(app.clone())
            .oneshot(req("GET", "/?t=tok", "127.0.0.1:4000", false, false))
            .await
            .unwrap();
        let cookie = res
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"),
            "{cookie}"
        );
        let (status, _) = send(&app, req("GET", "/?t=tok", "127.0.0.1:4000", false, false)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "second use refused");
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
}
