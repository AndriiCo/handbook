//! Blocking single-URL fetch (pure Rust: `ureq` + `rustls`).
//!
//! One GET with a caller timeout and a body cap, plus a minimal
//! robots.txt gate for third-party hosts (F1: re-checked each crawl).
//! No retries, no redirects policy beyond `ureq` defaults. Fetch
//! failures are data for an F1 `task(kind='validate_fact')`, never
//! silent skips (see `xtask fetch-url`).

use std::fmt;
use std::time::Duration;

/// Largest fetch body kept in memory (8 MiB). Anything larger is
/// rejected: the thin watch has no paging story yet.
pub const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;

/// One fetched URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub url: String,
    pub status: u16,
    pub bytes: Vec<u8>,
}

/// Fetch failure kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// DNS/TLS/connection/timeout failures.
    Transport(String),
    /// Non-2xx status (body already drained).
    HttpStatus(u16),
    /// Body exceeded [`MAX_BODY_BYTES`].
    TooLarge,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Transport(msg) => write!(f, "fetch transport error: {msg}"),
            FetchError::HttpStatus(status) => write!(f, "fetch HTTP status: {status}"),
            FetchError::TooLarge => write!(f, "fetch body exceeds {MAX_BODY_BYTES} bytes"),
        }
    }
}

impl std::error::Error for FetchError {}

/// GET `url`, waiting at most `timeout` for the whole call.
///
/// # Errors
///
/// [`FetchError::Transport`] on connection/TLS/timeout failure,
/// [`FetchError::HttpStatus`] on non-2xx status,
/// [`FetchError::TooLarge`] when the body exceeds [`MAX_BODY_BYTES`].
pub fn fetch_url(url: &str, timeout: Duration) -> Result<Fetched, FetchError> {
    install_crypto_provider();
    let agent = agent_with_timeout(timeout);
    let mut response = agent
        .get(url)
        .header(
            "User-Agent",
            "handbook-watch/0.1 (+https://andrii.ai/apostille)",
        )
        .call()
        .map_err(|e| map_ureq_error(e, url))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(FetchError::HttpStatus(status));
    }
    use std::io::Read as _;
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|e| FetchError::Transport(format!("{url}: read body: {e}")))?;
    if body.len() as u64 > MAX_BODY_BYTES {
        return Err(FetchError::TooLarge);
    }
    Ok(Fetched {
        url: url.to_string(),
        status,
        bytes: body,
    })
}

fn map_ureq_error(e: ureq::Error, url: &str) -> FetchError {
    match e {
        ureq::Error::StatusCode(status) => FetchError::HttpStatus(status),
        other => FetchError::Transport(format!("{url}: {other}")),
    }
}

fn agent_with_timeout(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build();
    ureq::Agent::new_with_config(config)
}

/// Install the workspace's single rustls provider (aws-lc-rs).
/// ureq runs with `rustls-no-provider` (its default would pull ring as a
/// second provider and rustls panics), so no default is installed for us.
/// First install wins; a prior install is fine, not an error.
fn install_crypto_provider() {
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let _ = rustls::crypto::CryptoProvider::install_default(provider);
}

/// Minimal robots.txt verdict for one URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotsVerdict {
    /// False only when a `User-agent: *` (or handbook) group says
    /// `Disallow: /`. Everything else fails open (thin slice: the crawl
    /// must not wedge on robots), with the reason kept in [`Self::text`].
    pub allowed: bool,
    /// Short reason: `allowed`, `disallow-all`, `no-robots`, or
    /// `unparseable-uri`. Callers store it in `source.robots`.
    pub text: String,
}

/// Fetch `<scheme>://<host>/robots.txt` for `url` and honor a root
/// disallow. Never fails: unparseable URLs, missing files, and fetch
/// errors all fail open.
pub fn check_robots(url: &str, timeout: Duration) -> RobotsVerdict {
    install_crypto_provider();
    let Some(root) = robots_root(url) else {
        return RobotsVerdict {
            allowed: true,
            text: "unparseable-uri".to_string(),
        };
    };
    let agent = agent_with_timeout(timeout);
    let body = agent
        .get(format!("{root}/robots.txt"))
        .header("User-Agent", "handbook-watch/0.1")
        .call()
        .ok()
        .and_then(|mut response| response.body_mut().read_to_string().ok())
        .unwrap_or_default();
    if body.is_empty() {
        return RobotsVerdict {
            allowed: true,
            text: "no-robots".to_string(),
        };
    }
    let allowed = !robots_disallows_all(&body);
    RobotsVerdict {
        allowed,
        text: if allowed { "allowed" } else { "disallow-all" }.to_string(),
    }
}

/// `scheme://host[:port]` prefix of an http(s) URL, or `None`.
fn robots_root(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let host = rest.split('/').next()?;
    if host.is_empty() {
        return None;
    }
    let scheme = if url.starts_with("https") {
        "https"
    } else {
        "http"
    };
    Some(format!("{scheme}://{host}"))
}

/// True when a `User-agent: *` (or handbook) group contains `Disallow: /`.
fn robots_disallows_all(body: &str) -> bool {
    let mut applies = false;
    for line in body.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        if key == "user-agent" {
            let agent = value.to_ascii_lowercase();
            applies = agent == "*" || agent.contains("handbook");
        } else if key == "disallow" && applies && value == "/" {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serve one canned HTTP/1.0 reply, then exit. Binds port 0.
    fn serve_once(
        status_line: &'static str,
        body: &'static [u8],
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let url = format!(
            "http://127.0.0.1:{}/",
            listener.local_addr().unwrap().port()
        );
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let head = format!(
                "{status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).expect("write head");
            stream.write_all(body).expect("write body");
        });
        (url, handle)
    }

    #[test]
    fn fetches_canned_bytes_from_loopback() {
        let (url, server) = serve_once("HTTP/1.0 200 OK", b"PO Box 40220");
        let fetched = fetch_url(&url, Duration::from_secs(5)).expect("fetch");
        assert_eq!(fetched.url, url);
        assert_eq!(fetched.status, 200);
        assert_eq!(fetched.bytes, b"PO Box 40220");
        server.join().unwrap();
    }

    #[test]
    fn non_2xx_is_an_error() {
        let (url, server) = serve_once("HTTP/1.0 404 Not Found", b"missing");
        let err = fetch_url(&url, Duration::from_secs(5)).expect_err("404 must fail");
        assert_eq!(err, FetchError::HttpStatus(404));
        server.join().unwrap();
    }

    #[test]
    fn refused_connection_is_transport() {
        // Port 0 above hands out a free port; binding then dropping a
        // listener reserves one nothing answers on.
        let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let err = fetch_url(&format!("http://127.0.0.1:{port}/"), Duration::from_secs(5))
            .expect_err("refused must fail");
        assert!(matches!(err, FetchError::Transport(_)), "got: {err}");
    }

    #[test]
    fn robots_root_shapes() {
        assert_eq!(
            robots_root("https://example.gov:8443/some/page"),
            Some("https://example.gov:8443".to_string())
        );
        assert_eq!(
            robots_root("http://127.0.0.1:9/"),
            Some("http://127.0.0.1:9".to_string())
        );
        assert_eq!(robots_root("not a url"), None);
        assert_eq!(robots_root("ftp://example.gov/"), None);
    }

    #[test]
    fn robots_disallow_parsing() {
        assert!(robots_disallows_all("User-agent: *\nDisallow: /"));
        assert!(robots_disallows_all(
            "# comment\nUser-agent: handbook-watch/0.1\nDisallow: / # all"
        ));
        assert!(!robots_disallows_all("User-agent: *\nDisallow: /tmp"));
        assert!(!robots_disallows_all("User-agent: otherbot\nDisallow: /"));
        assert!(!robots_disallows_all(""));
    }

    #[test]
    fn robots_disallow_all_blocks() {
        let (url, server) = serve_once("HTTP/1.0 200 OK", b"User-agent: *\nDisallow: /");
        let verdict = check_robots(&url, Duration::from_secs(5));
        assert!(!verdict.allowed, "got: {verdict:?}");
        assert_eq!(verdict.text, "disallow-all");
        server.join().unwrap();
    }

    #[test]
    fn robots_fail_open() {
        let (url, server) = serve_once("HTTP/1.0 200 OK", b"User-agent: *\nDisallow: /tmp");
        let verdict = check_robots(&url, Duration::from_secs(5));
        assert!(verdict.allowed, "got: {verdict:?}");
        server.join().unwrap();

        let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let verdict = check_robots(&format!("http://127.0.0.1:{port}/"), Duration::from_secs(5));
        assert!(verdict.allowed, "got: {verdict:?}");
        assert_eq!(verdict.text, "no-robots");

        let verdict = check_robots("not a url", Duration::from_secs(5));
        assert!(verdict.allowed, "got: {verdict:?}");
        assert_eq!(verdict.text, "unparseable-uri");
    }
}
