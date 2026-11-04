//! Worker enrolment and the token file (`docs/specs/worker-enrolment.md`
//! §2, §3, §8).
//!
//! **Token file** (`~/.config/apostille/worker.token`, or
//! `--token-file`): one JSON object,
//!
//! ```json
//! {"worker": "chandra@plant 2026-11-04", "token": "w2.<32 hex>.<64 hex>"}
//! ```
//!
//! `worker` is the name staff approved; `hello.worker` must equal it.
//! The file is 0600 in a 0700 directory, written through a temp file,
//! fsync and rename; a file other users can read is refused. A missing
//! file means "enrol".
//!
//! **Enrolment state** (`worker.enrol` next to the token file, 0600):
//! `{"id", "poll_secret", "expires_at", "code", "poll_after_secs"}`, so a
//! cron restart resumes polling the same request instead of opening a
//! new one (the server allows 3 requests per IP per hour). `code` is kept
//! only so a resumed run can log it again. A `410`, a `404` or a passed
//! `expires_at` deletes the file and the worker exits; the next cron
//! start opens a fresh request.

use std::io::Write as _;
use std::net::ToSocketAddrs as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::check_private;

/// The enrol route, on the same host as the WebSocket.
const ENROL_PATH: &str = "/apostille/api/v1/worker/enrol";
/// The code is logged once per request and again this often.
const LOG_EVERY_SECS: i64 = 300;
/// Polling interval when the server does not say.
const DEFAULT_POLL_SECS: u64 = 5;

/// The worker's credential, as stored in the token file. Debug never
/// prints the token.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    pub worker: String,
    pub token: String,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("worker", &self.worker)
            .finish_non_exhaustive()
    }
}

impl Credential {
    /// A `w2.` token safe to put in a header, and a name `hello` can
    /// carry (no `:`, which splits `lease_owner`).
    fn check(&self) -> Result<(), String> {
        let token_ok = self.token.starts_with("w2.")
            && self.token.len() <= 256
            && self.token.bytes().all(|b| b.is_ascii_graphic());
        let name_ok = !self.worker.is_empty()
            && self.worker.len() <= 128
            && !self.worker.contains(':')
            && !self.worker.chars().any(char::is_control);
        match (token_ok, name_ok) {
            (true, true) => Ok(()),
            (false, _) => Err("the token is not a w2. worker token".to_string()),
            (_, false) => Err(format!("bad worker name {:?}", self.worker)),
        }
    }
}

/// The token file's credential; `None` when there is no file (enrol).
pub fn load_credential(path: &Path) -> Result<Option<Credential>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    check_private(path)?;
    let credential: Credential = serde_json::from_str(&text)
        .map_err(|e| format!("{}: not a worker token file ({e})", path.display()))?;
    credential
        .check()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(credential))
}

/// Write the token file atomically (0600, directory 0700).
pub fn save_credential(path: &Path, credential: &Credential) -> Result<(), String> {
    credential.check()?;
    let bytes = serde_json::to_vec(credential).expect("credential serializes");
    write_private(path, &bytes)
}

/// Move a refused token file to `<name>.revoked` (replacing an older
/// one), so the next start enrols. Returns the new path.
pub fn retire_credential(path: &Path) -> Result<PathBuf, String> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".revoked");
    let to = path.with_file_name(name);
    std::fs::rename(path, &to).map_err(|e| format!("rename {}: {e}", path.display()))?;
    Ok(to)
}

/// `<dir of the token file>/worker.enrol`.
pub fn state_path(token_path: &Path) -> PathBuf {
    token_path.with_file_name("worker.enrol")
}

/// Write `bytes` to `path` readable by its owner only: a temp file in
/// the same directory (created 0700), fsync, rename, fsync the dir.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    private_dir(dir)?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(path.file_name().unwrap_or_default());
    tmp_name.push(format!(".tmp-{}", std::process::id()));
    let tmp = dir.join(tmp_name);
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let written = options.open(&tmp).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    let renamed = written.and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = renamed {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("write {}: {e}", path.display()));
    }
    // The rename is durable once the directory entry is.
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Create `dir` (and parents) and make it 0700.
fn private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("chmod 700 {}: {e}", dir.display()))?;
        }
    }
    Ok(())
}

/// The enrol URL for a `--connect` socket URL: `wss://` → `https://`
/// on the same host and port; `ws://` → `http://`, loopback only (the
/// same rule as the socket: no clear text off this machine).
pub fn enrol_url(connect: &str) -> Result<String, String> {
    let uri: tungstenite::http::Uri = connect
        .parse()
        .map_err(|e| format!("bad url {connect}: {e}"))?;
    let scheme = match uri.scheme_str() {
        Some("wss") => "https",
        Some("ws") => "http",
        _ => return Err(format!("url must be ws:// or wss://, got {connect}")),
    };
    let authority = uri
        .authority()
        .ok_or_else(|| format!("url has no host: {connect}"))?;
    if authority.as_str().contains('@') {
        return Err(format!("url must not carry credentials: {connect}"));
    }
    if scheme == "http" {
        let host = authority.host();
        let port = authority.port_u16().unwrap_or(80);
        let addrs: Vec<_> = (host.trim_matches(['[', ']']), port)
            .to_socket_addrs()
            .map_err(|e| format!("resolve {host}: {e}"))?
            .collect();
        if addrs.is_empty() || addrs.iter().any(|a| !a.ip().is_loopback()) {
            return Err(format!(
                "ws:// would enrol in clear text with {host}; use wss:// (ws:// is for loopback only)"
            ));
        }
    }
    Ok(format!("{scheme}://{authority}{ENROL_PATH}"))
}

/// The proposed `username@hostname` (spec §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    pub username: String,
    pub hostname: String,
}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}", self.username, self.hostname)
    }
}

impl Name {
    /// From raw OS values: lowercased, the host cut at its first `.`,
    /// characters outside `[a-z0-9._-]` replaced by `-`, cut to the
    /// rule's lengths (32 and 63).
    pub fn from_raw(user: &str, host: &str) -> Self {
        let short = host.trim().split('.').next().unwrap_or_default();
        Self {
            username: name_part(user, 32, "worker"),
            hostname: name_part(short, 63, "localhost"),
        }
    }
}

fn name_part(raw: &str, max: usize, fallback: &str) -> String {
    let part: String = raw
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(max)
        .collect();
    if part.is_empty() {
        fallback.to_string()
    } else {
        part
    }
}

/// `$USER` else `id -un`; the host from `hostname` else `uname -n`.
pub fn proposed_name() -> Name {
    let output = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let user = std::env::var("USER")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .or_else(|| output("id", &["-un"]))
        .unwrap_or_default();
    let host = output("hostname", &[])
        .or_else(|| output("uname", &["-n"]))
        .unwrap_or_default();
    Name::from_raw(&user, &host)
}

/// What the worker sends with its request.
pub struct Request {
    /// [`enrol_url`] of `--connect`.
    pub url: String,
    pub token_path: PathBuf,
    pub name: Name,
    pub version: String,
    pub started_at: i64,
    pub capabilities: Vec<String>,
}

/// The two public endpoints, behind a trait so tests can script them.
pub trait Http {
    /// POST a JSON body; `(status, JSON body or Null)`. `Err` is a
    /// transport failure.
    fn post(&self, url: &str, body: &Value) -> Result<(u16, Value), String>;
    fn sleep(&self, span: Duration) {
        std::thread::sleep(span);
    }
    fn now(&self) -> i64 {
        crate::unix_now()
    }
}

/// `ureq` with HTTP statuses as values (the workspace's aws-lc-rs).
pub struct UreqHttp {
    agent: ureq::Agent,
}

impl UreqHttp {
    pub fn new() -> Self {
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        let _ = rustls::crypto::CryptoProvider::install_default(provider);
        Self {
            agent: ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(20)))
                .build()
                .into(),
        }
    }
}

impl Http for UreqHttp {
    fn post(&self, url: &str, body: &Value) -> Result<(u16, Value), String> {
        let mut res = self
            .agent
            .post(url)
            .send_json(body)
            .map_err(|e| format!("{url}: {e}"))?;
        let status = res.status().as_u16();
        let text = res
            .body_mut()
            .with_config()
            .limit(64 * 1024)
            .read_to_string()
            .unwrap_or_default();
        Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
    }
}

/// `worker.enrol`: one open request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct State {
    id: String,
    poll_secret: String,
    expires_at: i64,
    #[serde(default)]
    code: String,
    #[serde(default = "default_poll")]
    poll_after_secs: u64,
}

fn default_poll() -> u64 {
    DEFAULT_POLL_SECS
}

fn load_state(path: &Path) -> Result<Option<State>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    check_private(path)?;
    match serde_json::from_str::<State>(&text) {
        Ok(state) if is_hex(&state.id, 32) && is_hex(&state.poll_secret, 64) => Ok(Some(state)),
        _ => {
            // Unreadable state cannot be resumed: start over.
            eprintln!(
                "worker: {} is not an enrol request; starting a new one",
                path.display()
            );
            let _ = std::fs::remove_file(path);
            Ok(None)
        }
    }
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The server's `{"error": …}`, for a log line.
fn error_of(reply: &Value) -> String {
    reply["error"]
        .as_str()
        .or_else(|| reply["status"].as_str())
        .unwrap_or("no detail")
        .chars()
        .take(200)
        .collect()
}

/// Open a request: a fresh poll secret, its hash to the server, the
/// answer saved to `worker.enrol`.
fn open(req: &Request, http: &dyn Http, state_file: &Path) -> Result<State, String> {
    let secret: [u8; 32] = rand::random();
    let body = json!({
        "username": req.name.username,
        "hostname": req.name.hostname,
        "version": req.version,
        "started_at": req.started_at,
        "capabilities": req.capabilities,
        "poll_hash": handbook_store::store::sha256_hex(&secret),
    });
    let (status, reply) = http
        .post(&req.url, &body)
        .map_err(|e| format!("enrol: {e}"))?;
    match status {
        200 | 202 => {}
        429 => {
            return Err(format!(
                "enrol refused: too many requests ({}); a later start tries again",
                error_of(&reply)
            ));
        }
        503 => return Err("enrolment is off on the server (worker.enrolment)".to_string()),
        _ => {
            return Err(format!(
                "enrol refused: HTTP {status} ({})",
                error_of(&reply)
            ));
        }
    }
    let id = reply["id"].as_str().unwrap_or_default();
    let code = reply["code"].as_str().unwrap_or_default();
    let expires_at = reply["expires_at"].as_i64().unwrap_or(0);
    if !is_hex(id, 32) || code.is_empty() || code.len() > 16 || expires_at <= 0 {
        return Err(format!(
            "enrol: unexpected answer (id {id:?}, code length {})",
            code.len()
        ));
    }
    let state = State {
        id: id.to_string(),
        poll_secret: hex(&secret),
        expires_at,
        code: code.to_string(),
        poll_after_secs: reply["poll_after_secs"]
            .as_u64()
            .unwrap_or(DEFAULT_POLL_SECS)
            .clamp(1, 60),
    };
    let bytes = serde_json::to_vec(&state).expect("state serializes");
    write_private(state_file, &bytes)?;
    Ok(state)
}

/// Enrol (or resume the open request) and poll until staff decide.
/// `Ok` once the token is saved; `Err` says why the worker should exit
/// (refused, denied, expired), with `worker.enrol` removed when the
/// request is over.
pub fn enrol(req: &Request, http: &dyn Http) -> Result<Credential, String> {
    let state_file = state_path(&req.token_path);
    let state = match load_state(&state_file)? {
        Some(state) if state.expires_at > http.now() => {
            eprintln!("worker: resuming enrol request {}", state.id);
            state
        }
        Some(_) => {
            let _ = std::fs::remove_file(&state_file);
            open(req, http, &state_file)?
        }
        None => open(req, http, &state_file)?,
    };
    let poll_url = format!("{}/{}/poll", req.url, state.id);
    let mut logged_at: Option<i64> = None;
    let mut transport_errors = 0u32;
    loop {
        let now = http.now();
        if now >= state.expires_at {
            let _ = std::fs::remove_file(&state_file);
            return Err(format!(
                "enrol request {} expired before approval; the next start opens a new one",
                state.id
            ));
        }
        if logged_at.is_none_or(|at| now - at >= LOG_EVERY_SECS) {
            eprintln!(
                "enrol code {} for {}: approve it on Work queue → Workers",
                state.code, req.name
            );
            logged_at = Some(now);
        }
        http.sleep(Duration::from_secs(state.poll_after_secs));
        let answer = http.post(&poll_url, &json!({ "poll_secret": state.poll_secret }));
        let (status, reply) = match answer {
            Ok(answer) => answer,
            Err(e) => {
                transport_errors += 1;
                if transport_errors == 1 || transport_errors.is_multiple_of(12) {
                    eprintln!("worker: enrol poll: {e}; still trying");
                }
                continue;
            }
        };
        transport_errors = 0;
        match (status, reply["status"].as_str()) {
            (200, Some("pending")) => {}
            (200, Some("approved")) => {
                let credential = Credential {
                    worker: reply["worker"].as_str().unwrap_or_default().to_string(),
                    token: reply["token"].as_str().unwrap_or_default().to_string(),
                };
                let saved = save_credential(&req.token_path, &credential);
                // Delivered once either way: this request is over.
                let _ = std::fs::remove_file(&state_file);
                saved.map_err(|e| format!("enrol approved but the token was not saved: {e}"))?;
                eprintln!(
                    "worker: enrolled as {}; token saved to {}",
                    credential.worker,
                    req.token_path.display()
                );
                return Ok(credential);
            }
            (410, _) => {
                let _ = std::fs::remove_file(&state_file);
                return Err(format!(
                    "enrol request {} is {}; the next start opens a new one",
                    state.id,
                    error_of(&reply)
                ));
            }
            (404, _) => {
                let _ = std::fs::remove_file(&state_file);
                return Err(format!(
                    "the server does not know enrol request {}; the next start opens a new one",
                    state.id
                ));
            }
            _ => eprintln!(
                "worker: enrol poll answered HTTP {status} ({}); still trying",
                error_of(&reply)
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const TOKEN: &str = "w2.0123456789abcdef0123456789abcdef.0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn contract() -> Value {
        serde_json::from_str(include_str!("../../../docs/contracts/worker-frames.json")).unwrap()
    }

    /// Scripted answers, in order; every request is recorded.
    struct Script {
        answers: Mutex<Vec<(u16, Value)>>,
        seen: Mutex<Vec<(String, Value)>>,
        now: i64,
    }

    impl Script {
        fn new(now: i64, answers: Vec<(u16, Value)>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().rev().collect()),
                seen: Mutex::new(Vec::new()),
                now,
            }
        }
        fn seen(&self) -> Vec<(String, Value)> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Http for Script {
        fn post(&self, url: &str, body: &Value) -> Result<(u16, Value), String> {
            self.seen
                .lock()
                .unwrap()
                .push((url.to_string(), body.clone()));
            self.answers
                .lock()
                .unwrap()
                .pop()
                .ok_or_else(|| "script ran out".to_string())
        }
        fn sleep(&self, _span: Duration) {}
        fn now(&self) -> i64 {
            self.now
        }
    }

    fn request(dir: &Path) -> Request {
        Request {
            url: "https://andrii.ai/apostille/api/v1/worker/enrol".to_string(),
            token_path: dir.join("cfg/worker.token"),
            name: Name::from_raw("Chandra", "plant.lan"),
            version: "0.1.0+1bda381".to_string(),
            started_at: 1_793_750_000,
            capabilities: vec!["scrape.urls".to_string()],
        }
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn enrol_url_follows_the_socket_url() {
        assert_eq!(
            enrol_url("wss://andrii.ai/apostille/api/v1/worker/ws").unwrap(),
            "https://andrii.ai/apostille/api/v1/worker/enrol"
        );
        assert_eq!(
            enrol_url("wss://staging.example.test:8443/apostille/api/v1/worker/ws").unwrap(),
            "https://staging.example.test:8443/apostille/api/v1/worker/enrol"
        );
        assert_eq!(
            enrol_url("ws://127.0.0.1:18318/apostille/api/v1/worker/ws").unwrap(),
            "http://127.0.0.1:18318/apostille/api/v1/worker/enrol"
        );
        // 192.0.2.1 is TEST-NET-1: refused before any packet is sent.
        let err = enrol_url("ws://192.0.2.1:9/apostille/api/v1/worker/ws").unwrap_err();
        assert!(err.contains("wss://"), "{err}");
        for bad in [
            "https://andrii.ai/x",
            "wss://user:pw@andrii.ai/x",
            "not a url",
        ] {
            assert!(enrol_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn proposed_names_follow_the_rule() {
        let name = Name::from_raw("Chandra", "Plant.LAN");
        assert_eq!(name.to_string(), "chandra@plant");
        assert_eq!(
            Name::from_raw(" dev user ", "my box.local").to_string(),
            "dev-user@my-box"
        );
        assert_eq!(Name::from_raw("", "").to_string(), "worker@localhost");
        assert_eq!(Name::from_raw(&"a".repeat(40), "h").username.len(), 32);
        let rule = regex::Regex::new(r"^[a-z0-9._-]{1,32}@[a-z0-9._-]{1,63}$").unwrap();
        let here = proposed_name().to_string();
        assert!(rule.is_match(&here), "{here}");
    }

    #[cfg(unix)]
    #[test]
    fn token_file_round_trips_privately_and_refuses_a_wide_file() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg/worker.token");
        assert_eq!(load_credential(&path).unwrap(), None, "missing: enrol");
        let credential = Credential {
            worker: "chandra@plant 2026-11-04".to_string(),
            token: TOKEN.to_string(),
        };
        save_credential(&path, &credential).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        let text = std::fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v,
            json!({"worker": "chandra@plant 2026-11-04", "token": TOKEN})
        );
        assert_eq!(load_credential(&path).unwrap(), Some(credential.clone()));
        assert!(!format!("{credential:?}").contains("w2."), "Debug hides it");

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(load_credential(&path).unwrap_err().contains("chmod 600"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        std::fs::write(&path, "APOSTILLE_WORKER_TOKEN=x\n").unwrap();
        assert!(
            load_credential(&path).is_err(),
            "an env file is not a token file"
        );

        save_credential(&path, &credential).unwrap();
        let moved = retire_credential(&path).unwrap();
        assert_eq!(moved, dir.path().join("cfg/worker.token.revoked"));
        assert_eq!(load_credential(&path).unwrap(), None, "enrols again");
    }

    /// A fresh request: the hash the server gets is SHA-256 of the raw
    /// secret the polls carry; approval saves the token and removes
    /// `worker.enrol`. Uses the contract samples as the server's answers.
    #[test]
    fn enrols_polls_and_saves_the_token() {
        let seed = contract();
        let enrol_samples = &seed["enrol"];
        let dir = tempfile::tempdir().unwrap();
        let req = request(dir.path());
        let http = Script::new(
            1_793_750_100,
            vec![
                (202, enrol_samples["response"].clone()),
                (200, enrol_samples["poll_pending"].clone()),
                (200, enrol_samples["poll_approved"].clone()),
            ],
        );
        let credential = enrol(&req, &http).unwrap();
        assert_eq!(credential.worker, "chandra@plant 2026-11-04");
        assert_eq!(credential.token, enrol_samples["poll_approved"]["token"]);
        assert_eq!(load_credential(&req.token_path).unwrap(), Some(credential));
        assert!(!state_path(&req.token_path).exists());
        let seen = http.seen();
        assert_eq!(seen.len(), 3);
        let (url, body) = &seen[0];
        assert_eq!(url, &req.url);
        let keys = |v: &Value| {
            let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
            k.sort();
            k
        };
        assert_eq!(keys(body), keys(&enrol_samples["request"]));
        assert_eq!(body["username"], "chandra");
        assert_eq!(body["hostname"], "plant");
        let poll_url = format!("{}/5f2c1d0e9b8a7f6e5d4c3b2a1f0e9d8c/poll", req.url);
        for (url, poll) in &seen[1..] {
            assert_eq!(url, &poll_url);
            assert_eq!(keys(poll), keys(&enrol_samples["poll_request"]));
            let secret = unhex(poll["poll_secret"].as_str().unwrap());
            assert_eq!(secret.len(), 32);
            assert_eq!(
                body["poll_hash"].as_str().unwrap(),
                handbook_store::store::sha256_hex(&secret)
            );
        }
    }

    /// A restart while a request is open keeps polling it: no new
    /// request (one pending row, acceptance 2).
    #[cfg(unix)]
    #[test]
    fn a_restart_resumes_the_open_request() {
        let seed = contract();
        let dir = tempfile::tempdir().unwrap();
        let req = request(dir.path());
        let first = Script::new(
            1_793_750_100,
            vec![(202, seed["enrol"]["response"].clone())],
        );
        // The first run opens the request, then dies before approval.
        let state_file = state_path(&req.token_path);
        let opened = open(&req, &first, &state_file).unwrap();
        assert!(state_file.is_file(), "the request survives the restart");
        assert_eq!(mode(&state_file), 0o600);
        assert_eq!(opened.code, "K7QF-2M");
        let secret = json!(opened.poll_secret);

        let second = Script::new(
            1_793_750_200,
            vec![(200, seed["enrol"]["poll_approved"].clone())],
        );
        let credential = enrol(&req, &second).unwrap();
        assert_eq!(credential.worker, "chandra@plant 2026-11-04");
        let seen = second.seen();
        assert_eq!(seen.len(), 1, "no second enrol request");
        assert!(
            seen[0]
                .0
                .ends_with("/5f2c1d0e9b8a7f6e5d4c3b2a1f0e9d8c/poll")
        );
        assert_eq!(seen[0].1["poll_secret"], secret, "the same poll secret");
        assert!(!state_file.exists());
    }

    #[test]
    fn a_gone_or_expired_request_is_dropped() {
        let seed = contract();
        let dir = tempfile::tempdir().unwrap();
        let req = request(dir.path());
        let http = Script::new(
            1_793_750_100,
            vec![
                (202, seed["enrol"]["response"].clone()),
                (410, seed["enrol"]["poll_gone"].clone()),
            ],
        );
        let err = enrol(&req, &http).unwrap_err();
        assert!(err.contains("denied"), "{err}");
        assert!(!state_path(&req.token_path).exists());
        assert_eq!(load_credential(&req.token_path).unwrap(), None);

        // A saved request past its expiry is not polled: a new one opens.
        let stale = State {
            id: "a".repeat(32),
            poll_secret: "b".repeat(64),
            expires_at: 100,
            code: "AAAA-AA".to_string(),
            poll_after_secs: 5,
        };
        write_private(
            &state_path(&req.token_path),
            &serde_json::to_vec(&stale).unwrap(),
        )
        .unwrap();
        let http = Script::new(1_793_750_100, vec![(429, json!({"error": "too many"}))]);
        let err = enrol(&req, &http).unwrap_err();
        assert!(err.contains("too many requests"), "{err}");
        assert_eq!(http.seen()[0].0, req.url, "a fresh request, not a poll");
        assert!(!state_path(&req.token_path).exists());
    }
}
