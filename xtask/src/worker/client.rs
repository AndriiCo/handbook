//! Worker side of the queue socket (frames: `docs/specs/work-queue.md`
//! §2). Blocking `tungstenite` over our own TCP/rustls stream (the
//! workspace's aws-lc-rs provider, webpki roots): the socket polls with
//! a 1 s read timeout so one thread can heartbeat while the job runs on
//! another. A dropped socket never kills the job: the next session
//! re-sends `hello`, keeps heartbeating with the same lease token, and
//! uploads the result when it is ready.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{ClientRequestBuilder, Message, WebSocket};

use super::{JobContext, JobError, JobHandler, JobOutput, WorkerEnv, run_job};

/// Socket poll granularity (heartbeat and pong latency bound).
const POLL: Duration = Duration::from_secs(1);
/// Connect + handshake + `welcome` budget.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Reply budget for small frames (checkout, heartbeat).
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// Reply budget after a result upload (server writes the blob).
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(180);
/// Largest frame we accept or send (matches the server cap).
const MAX_FRAME: usize = 24 * 1024 * 1024;
/// Reconnect backoff ceiling.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

pub struct Config {
    pub url: String,
    pub token: String,
    pub name: String,
    pub workdir: PathBuf,
    pub cache_dir: PathBuf,
    pub repo: PathBuf,
    pub env: WorkerEnv,
    pub gpu_lock: PathBuf,
    /// Exit after the first finished job (smoke runs, tests).
    pub once: bool,
}

/// Run until killed (or, with `once`, until one job is acknowledged).
/// Tests pass fake handlers; `cargo xtask worker` passes [`super::handlers`].
pub fn run_with(cfg: &Config, handlers: Arc<Vec<Box<dyn JobHandler>>>) {
    let caps: Vec<&'static str> = handlers
        .iter()
        .filter(|h| h.available(&cfg.env))
        .map(|h| h.kind())
        .collect();
    let mut running: Option<Running> = None;
    let mut backoff = Backoff::default();
    loop {
        match session(cfg, &handlers, &caps, &mut running, &mut backoff) {
            Ok(()) => return,
            Err(e) => {
                let wait = backoff.next_delay();
                eprintln!("worker: {e}; reconnecting in {}s", wait.as_secs());
                std::thread::sleep(wait);
            }
        }
    }
}

/// Exponential backoff with jitter: 1 s, 2 s, 4 s … capped at 60 s.
#[derive(Default)]
struct Backoff {
    failures: u32,
}

impl Backoff {
    fn next_delay(&mut self) -> Duration {
        let base = Duration::from_secs(1u64 << self.failures.min(6)).min(MAX_BACKOFF);
        self.failures = self.failures.saturating_add(1);
        base + Duration::from_millis(rand::random::<u64>() % 500)
    }

    fn reset(&mut self) {
        self.failures = 0;
    }
}

/// A job on its own thread, plus the lease that owns it.
struct Running {
    id: i64,
    kind: String,
    lease: String,
    dir: PathBuf,
    rx: mpsc::Receiver<Result<JobOutput, JobError>>,
    last_beat: Instant,
    /// The server said the lease is gone: finish, but upload nothing.
    lost: bool,
    /// The finished job's `complete`/`fail` frame (and whether it is a
    /// success), kept until the server answers it: a socket that dies
    /// mid-upload re-sends the same frame from the next session.
    upload: Option<(bool, Value)>,
}

impl Running {
    fn start(cfg: &Config, handlers: &Arc<Vec<Box<dyn JobHandler>>>, job: &Value) -> Self {
        let id = job["id"].as_i64().unwrap_or(0);
        let kind = job["kind"].as_str().unwrap_or_default().to_string();
        let attempt = job["attempt"].as_i64().unwrap_or(1);
        let dir = cfg.workdir.join(format!("job-{id}-a{attempt}"));
        let ctx = JobContext {
            id,
            dir: dir.clone(),
            repo: cfg.repo.clone(),
            cache_dir: cfg.cache_dir.clone(),
            env: cfg.env.clone(),
        };
        let payload = job.get("payload").cloned().unwrap_or(Value::Null);
        let (tx, rx) = mpsc::channel();
        let handlers = Arc::clone(handlers);
        let gpu_lock = cfg.gpu_lock.clone();
        let thread_kind = kind.clone();
        std::thread::spawn(move || {
            let outcome = match handlers.iter().find(|h| h.kind() == thread_kind) {
                Some(handler) => run_job(handler.as_ref(), &ctx, &payload, &gpu_lock),
                None => Err(JobError::fatal(format!(
                    "this worker has no {thread_kind} handler"
                ))),
            };
            let _ = tx.send(outcome);
        });
        eprintln!(
            "worker: job {id} ({kind}) attempt {attempt} started in {}",
            dir.display()
        );
        Self {
            id,
            kind,
            lease: job["lease"].as_str().unwrap_or_default().to_string(),
            dir,
            rx,
            last_beat: Instant::now(),
            lost: false,
            upload: None,
        }
    }

    /// The outcome once the thread is done (a vanished thread fails).
    fn poll(&self) -> Option<Result<JobOutput, JobError>> {
        match self.rx.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(JobError::fatal("job thread ended without a result")))
            }
        }
    }
}

/// `complete` (with the base64 result) or `fail` for one outcome.
fn finish_frame(job: &Running, outcome: Result<JobOutput, JobError>, max_result: usize) -> Value {
    match outcome {
        Ok(out) if out.bytes.len() > max_result => json!({
            "type": "fail", "id": job.id, "lease": job.lease, "retryable": false,
            "error": format!("result is {} bytes, over the {max_result}-byte cap", out.bytes.len()),
        }),
        Ok(out) => {
            // Keep a local copy next to the job's scratch files.
            let _ = std::fs::write(
                job.dir.join(super::result_name(&out.content_type)),
                &out.bytes,
            );
            json!({
                "type": "complete", "id": job.id, "lease": job.lease,
                "content_type": out.content_type,
                "sha256": handbook_store::store::sha256_hex(&out.bytes),
                "result_b64": base64::engine::general_purpose::STANDARD.encode(&out.bytes),
                "summary": out.summary,
            })
        }
        Err(e) => json!({
            "type": "fail", "id": job.id, "lease": job.lease,
            "error": e.message, "retryable": e.retryable,
        }),
    }
}

/// One connection's lifetime. `Ok` only when `once` is satisfied.
fn session(
    cfg: &Config,
    handlers: &Arc<Vec<Box<dyn JobHandler>>>,
    caps: &[&'static str],
    running: &mut Option<Running>,
    backoff: &mut Backoff,
) -> Result<(), String> {
    let mut ws = connect(&cfg.url, &cfg.token)?;
    send(
        &mut ws,
        &json!({"type": "hello", "worker": cfg.name, "capabilities": caps,
                "version": env!("CARGO_PKG_VERSION")}),
    )?;
    let welcome = await_reply(&mut ws, CONNECT_TIMEOUT)?;
    if welcome["type"] != "welcome" {
        return Err(format!("server refused hello: {welcome}"));
    }
    backoff.reset();
    eprintln!("worker: connected to {} as {}", cfg.url, cfg.name);
    let heartbeat = Duration::from_secs(
        welcome["heartbeat_secs"]
            .as_u64()
            .unwrap_or(60)
            .clamp(1, 600),
    );
    let max_result = welcome["max_result_bytes"]
        .as_u64()
        .map_or(16 * 1024 * 1024, |n| n as usize);
    loop {
        let Some(job) = running.as_mut() else {
            send(&mut ws, &json!({"type": "checkout"}))?;
            let reply = await_reply(&mut ws, REPLY_TIMEOUT)?;
            match reply["type"].as_str() {
                Some("job") => *running = Some(Running::start(cfg, handlers, &reply)),
                Some("idle") => {
                    let secs = reply["retry_after_secs"]
                        .as_u64()
                        .unwrap_or(15)
                        .clamp(1, 300);
                    pump_for(&mut ws, Duration::from_secs(secs))?;
                }
                _ => return Err(format!("checkout answered {reply}")),
            }
            continue;
        };
        if job.upload.is_none()
            && let Some(outcome) = job.poll()
        {
            if job.lost {
                eprintln!(
                    "worker: job {} finished after its lease was lost; result dropped",
                    job.id
                );
                *running = None;
                if cfg.once {
                    return Ok(());
                }
                continue;
            }
            job.upload = Some((outcome.is_ok(), finish_frame(job, outcome, max_result)));
        }
        if let Some((ok, frame)) = &job.upload {
            send(&mut ws, frame)?;
            let reply = await_answer(&mut ws, UPLOAD_TIMEOUT)?;
            if reply["code"] == "store" {
                return Err(format!(
                    "job {}: the server could not store the result ({reply}); re-sending",
                    job.id
                ));
            }
            // `ack`, or a final refusal: `lease_lost` after a re-send
            // means an earlier upload landed and its ack was lost.
            eprintln!(
                "worker: job {} ({}) {} → {reply}",
                job.id,
                job.kind,
                if *ok { "complete" } else { "failed" }
            );
            *running = None;
            if cfg.once {
                return Ok(());
            }
            continue;
        }
        if job.last_beat.elapsed() >= heartbeat {
            send(
                &mut ws,
                &json!({"type": "heartbeat", "id": job.id, "lease": job.lease}),
            )?;
            job.last_beat = Instant::now();
        }
        if let Some(frame) = read_frame(&mut ws)?
            && frame["code"] == "lease_lost"
        {
            eprintln!("worker: job {} lease lost (expired or reassigned)", job.id);
            job.lost = true;
        }
    }
}

/// Read and discard frames for `span` (keeps pings answered).
fn pump_for(ws: &mut WebSocket<Stream>, span: Duration) -> Result<(), String> {
    let until = Instant::now() + span;
    while Instant::now() < until {
        read_frame(ws)?;
    }
    Ok(())
}

/// The server's answer to an upload within `budget`: the next frame
/// that is not a `lease` reply to a heartbeat still in flight.
fn await_answer(ws: &mut WebSocket<Stream>, budget: Duration) -> Result<Value, String> {
    let until = Instant::now() + budget;
    while Instant::now() < until {
        if let Some(frame) = read_frame(ws)?
            && frame["type"] != "lease"
        {
            return Ok(frame);
        }
    }
    Err(format!("no reply within {}s", budget.as_secs()))
}

/// Next JSON text frame within `budget`.
fn await_reply(ws: &mut WebSocket<Stream>, budget: Duration) -> Result<Value, String> {
    let until = Instant::now() + budget;
    while Instant::now() < until {
        if let Some(frame) = read_frame(ws)? {
            return Ok(frame);
        }
    }
    Err(format!("no reply within {}s", budget.as_secs()))
}

/// One poll: a JSON text frame, `None` on timeout or control frames,
/// `Err` when the socket is gone.
fn read_frame(ws: &mut WebSocket<Stream>) -> Result<Option<Value>, String> {
    match ws.read() {
        Ok(Message::Text(text)) => serde_json::from_str(text.as_str())
            .map(Some)
            .map_err(|e| format!("unparseable server frame: {e}")),
        Ok(Message::Close(_)) => Err("server closed the socket".to_string()),
        Ok(_) => {
            // Pongs to server pings queue inside tungstenite; flush them.
            ws.flush().map_err(|e| format!("socket: {e}"))?;
            Ok(None)
        }
        Err(tungstenite::Error::Io(e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            match ws.flush() {
                Ok(()) => Ok(None),
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), std::io::ErrorKind::WouldBlock) =>
                {
                    Ok(None)
                }
                Err(e) => Err(format!("socket: {e}")),
            }
        }
        Err(e) => Err(format!("socket: {e}")),
    }
}

fn send(ws: &mut WebSocket<Stream>, frame: &Value) -> Result<(), String> {
    ws.send(Message::text(frame.to_string()))
        .map_err(|e| format!("send: {e}"))
}

/// Plain TCP (`ws://`, tests and loopback) or rustls (`wss://`).
pub(super) enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Stream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(s) => s,
            Stream::Tls(s) => s.get_ref(),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            Stream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            Stream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(s) => s.flush(),
        }
    }
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("aws-lc-rs supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

/// Dial, (TLS,) and upgrade with the bearer token.
fn connect(url: &str, token: &str) -> Result<WebSocket<Stream>, String> {
    let uri: tungstenite::http::Uri = url.parse().map_err(|e| format!("bad url {url}: {e}"))?;
    let tls = match uri.scheme_str() {
        Some("wss") => true,
        Some("ws") => false,
        _ => return Err(format!("url must be ws:// or wss://, got {url}")),
    };
    let host = uri
        .host()
        .ok_or_else(|| format!("url has no host: {url}"))?
        .to_string();
    let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("resolve {host}: {e}"))?
        .next()
        .ok_or_else(|| format!("resolve {host}: no address"))?;
    if !tls && !addr.ip().is_loopback() {
        return Err(format!(
            "ws:// would send the token in clear text to {host}; use wss:// (ws:// is for loopback only)"
        ));
    }
    let tcp = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|e| format!("connect {host}:{port}: {e}"))?;
    tcp.set_read_timeout(Some(CONNECT_TIMEOUT))
        .and_then(|()| tcp.set_nodelay(true))
        .map_err(|e| format!("socket options: {e}"))?;
    let stream = if tls {
        let name = rustls::pki_types::ServerName::try_from(host.clone())
            .map_err(|e| format!("tls name {host}: {e}"))?;
        let conn =
            rustls::ClientConnection::new(tls_config(), name).map_err(|e| format!("tls: {e}"))?;
        Stream::Tls(Box::new(rustls::StreamOwned::new(conn, tcp)))
    } else {
        Stream::Plain(tcp)
    };
    let request =
        ClientRequestBuilder::new(uri).with_header("Authorization", format!("Bearer {token}"));
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let (ws, _) = tungstenite::client::client_with_config(request, stream, Some(config)).map_err(
        |e| match e {
            tungstenite::HandshakeError::Failure(tungstenite::Error::Http(res)) => {
                format!("upgrade refused: HTTP {}", res.status())
            }
            other => format!("handshake: {other}"),
        },
    )?;
    ws.get_ref()
        .tcp()
        .set_read_timeout(Some(POLL))
        .map_err(|e| format!("socket options: {e}"))?;
    Ok(ws)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    const TOKEN: &str = "client-test-token-0123456789abcdef0123";
    const LEASE: &str = "feedfacefeedfacefeedfacefeedface";

    struct Echo;

    impl JobHandler for Echo {
        fn kind(&self) -> &'static str {
            "test.echo"
        }
        fn run(&self, _ctx: &JobContext, payload: &Value) -> Result<JobOutput, JobError> {
            // Long enough that at least one heartbeat goes out.
            std::thread::sleep(Duration::from_millis(1500));
            Ok(JobOutput::json(payload, json!({"echoed": true})))
        }
    }

    struct Hidden;

    impl JobHandler for Hidden {
        fn kind(&self) -> &'static str {
            "test.hidden"
        }
        fn available(&self, _env: &WorkerEnv) -> bool {
            false
        }
        fn run(&self, _ctx: &JobContext, _payload: &Value) -> Result<JobOutput, JobError> {
            Err(JobError::fatal("never announced"))
        }
    }

    type ServerWs = tungstenite::WebSocket<TcpStream>;

    // tungstenite fixes the callback's error type (an HTTP response).
    #[allow(clippy::result_large_err)]
    fn accept(listener: &TcpListener) -> ServerWs {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        tungstenite::accept_hdr(
            stream,
            |req: &tungstenite::handshake::server::Request,
             res: tungstenite::handshake::server::Response| {
                let auth = req
                    .headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok());
                assert_eq!(auth, Some(format!("Bearer {TOKEN}").as_str()));
                Ok(res)
            },
        )
        .unwrap()
    }

    fn recv(ws: &mut ServerWs) -> Value {
        loop {
            match ws.read().unwrap() {
                Message::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
                _ => continue,
            }
        }
    }

    fn reply(ws: &mut ServerWs, frame: Value) {
        ws.send(Message::text(frame.to_string())).unwrap();
    }

    fn welcome(ws: &mut ServerWs) -> Value {
        let hello = recv(ws);
        assert_eq!(hello["type"], "hello");
        reply(
            ws,
            json!({"type": "welcome", "worker": "t", "lease_secs": 300, "heartbeat_secs": 1,
                   "max_result_bytes": 1024}),
        );
        hello
    }

    /// Connect, take a job, lose the socket mid-job, reconnect, keep
    /// the lease (heartbeat with the same token), upload the result.
    #[test]
    fn reconnects_mid_job_and_completes_with_the_same_lease() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "ws://{}/apostille/api/v1/worker/ws",
            listener.local_addr().unwrap()
        );
        let server = std::thread::spawn(move || {
            let mut first = accept(&listener);
            let hello = welcome(&mut first);
            assert_eq!(
                hello["capabilities"],
                json!(["test.echo"]),
                "hidden kinds stay quiet"
            );
            assert_eq!(recv(&mut first)["type"], "checkout");
            reply(
                &mut first,
                json!({"type": "job", "id": 9, "kind": "test.echo", "payload": {"x": 1},
                       "lease": LEASE, "lease_expires_at": 0, "attempt": 1}),
            );
            drop(first); // socket dies while the job runs
            let mut second = accept(&listener);
            welcome(&mut second);
            let mut beats = 0;
            let done = loop {
                let frame = recv(&mut second);
                match frame["type"].as_str() {
                    Some("heartbeat") => {
                        assert_eq!(
                            (frame["id"].as_i64(), frame["lease"].as_str()),
                            (Some(9), Some(LEASE))
                        );
                        beats += 1;
                        reply(
                            &mut second,
                            json!({"type": "lease", "id": 9, "lease_expires_at": 1}),
                        );
                    }
                    Some("complete") => break frame,
                    other => panic!("unexpected frame {other:?}"),
                }
            };
            assert_eq!(done["lease"], LEASE);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(done["result_b64"].as_str().unwrap())
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap(),
                json!({"x": 1})
            );
            assert_eq!(done["sha256"], handbook_store::store::sha256_hex(&bytes));
            assert_eq!(done["summary"]["echoed"], true);
            reply(
                &mut second,
                json!({"type": "ack", "id": 9, "status": "done"}),
            );
            beats
        });
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config {
            url,
            token: TOKEN.to_string(),
            name: "plant-test".to_string(),
            workdir: dir.path().join("work"),
            cache_dir: dir.path().to_path_buf(),
            repo: super::super::repo_root(),
            env: WorkerEnv::default(),
            gpu_lock: dir.path().join("gpu.lock"),
            once: true,
        };
        let handlers: Vec<Box<dyn JobHandler>> = vec![Box::new(Echo), Box::new(Hidden)];
        run_with(&cfg, Arc::new(handlers));
        let beats = server.join().unwrap();
        assert!(beats >= 1, "heartbeats keep the lease while the job runs");
        assert!(dir.path().join("work/job-9-a1/result.json").is_file());
    }

    /// The socket dies after the upload and before the ack: the next
    /// session re-sends the same result instead of losing it.
    #[test]
    fn keeps_the_result_until_the_server_answers_it() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "ws://{}/apostille/api/v1/worker/ws",
            listener.local_addr().unwrap()
        );
        let server = std::thread::spawn(move || {
            let until_complete = |ws: &mut ServerWs| loop {
                let frame = recv(ws);
                match frame["type"].as_str() {
                    Some("heartbeat") => {
                        reply(ws, json!({"type": "lease", "id": 4, "lease_expires_at": 1}))
                    }
                    Some("complete") => break frame,
                    other => panic!("unexpected frame {other:?}"),
                }
            };
            let mut first = accept(&listener);
            welcome(&mut first);
            assert_eq!(recv(&mut first)["type"], "checkout");
            reply(
                &mut first,
                json!({"type": "job", "id": 4, "kind": "test.echo", "payload": {"x": 2},
                       "lease": LEASE, "lease_expires_at": 0, "attempt": 1}),
            );
            let sent = until_complete(&mut first);
            // A heartbeat reply still in flight is not the answer.
            reply(
                &mut first,
                json!({"type": "lease", "id": 4, "lease_expires_at": 1}),
            );
            drop(first); // the ack never arrives
            let mut second = accept(&listener);
            welcome(&mut second);
            let resent = until_complete(&mut second);
            reply(
                &mut second,
                json!({"type": "ack", "id": 4, "status": "done"}),
            );
            (sent, resent)
        });
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config {
            url,
            token: TOKEN.to_string(),
            name: "plant-test".to_string(),
            workdir: dir.path().join("work"),
            cache_dir: dir.path().to_path_buf(),
            repo: super::super::repo_root(),
            env: WorkerEnv::default(),
            gpu_lock: dir.path().join("gpu.lock"),
            once: true,
        };
        let handlers: Vec<Box<dyn JobHandler>> = vec![Box::new(Echo)];
        run_with(&cfg, Arc::new(handlers));
        let (sent, resent) = server.join().unwrap();
        assert_eq!(sent, resent, "the same frame, lease and bytes");
        assert_eq!(resent["lease"], LEASE);
    }

    #[test]
    fn refuses_plain_ws_off_loopback() {
        // 192.0.2.1 is TEST-NET-1: refused before any packet is sent.
        let err = connect("ws://192.0.2.1:9/apostille/api/v1/worker/ws", TOKEN)
            .err()
            .expect("plain ws:// to a remote host");
        assert!(err.contains("wss://"), "{err}");
    }

    #[test]
    fn oversized_results_fail_instead_of_uploading() {
        let dir = tempfile::tempdir().unwrap();
        let (_tx, rx) = mpsc::channel();
        let job = Running {
            id: 3,
            kind: "test.echo".to_string(),
            lease: LEASE.to_string(),
            dir: dir.path().to_path_buf(),
            rx,
            last_beat: Instant::now(),
            lost: false,
            upload: None,
        };
        let big = JobOutput::json(&json!({"x": "y".repeat(100)}), Value::Null);
        let frame = finish_frame(&job, Ok(big), 16);
        assert_eq!(
            (frame["type"].as_str(), frame["retryable"].as_bool()),
            (Some("fail"), Some(false))
        );
        let failed = finish_frame(&job, Err(JobError::retry("rate limited")), 16);
        assert_eq!(failed["retryable"], true);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let mut b = Backoff::default();
        let secs: Vec<u64> = (0..9).map(|_| b.next_delay().as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
        b.reset();
        assert_eq!(b.next_delay().as_secs(), 1);
    }

    /// The frames this client builds carry the keys the shared contract
    /// samples carry (the server's tests parse the same file).
    #[test]
    fn client_frames_match_the_contract() {
        let seed: Value =
            serde_json::from_str(include_str!("../../../docs/contracts/worker-frames.json"))
                .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (_tx, rx) = mpsc::channel();
        let job = Running {
            id: 7,
            kind: "k".to_string(),
            lease: LEASE.to_string(),
            dir: dir.path().to_path_buf(),
            rx,
            last_beat: Instant::now(),
            lost: false,
            upload: None,
        };
        let complete = finish_frame(&job, Ok(JobOutput::json(&json!({}), json!({}))), 1024);
        let fail = finish_frame(&job, Err(JobError::retry("x")), 1024);
        let keys = |v: &Value| {
            let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
            k.sort();
            k
        };
        let samples = seed["client"].as_array().unwrap();
        let sample = |t: &str| samples.iter().find(|s| s["type"] == t).unwrap().clone();
        assert_eq!(keys(&complete), keys(&sample("complete")));
        assert_eq!(keys(&fail), keys(&sample("fail")));
    }
}
