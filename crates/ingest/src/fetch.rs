//! Blocking single-URL fetch (pure Rust: `ureq` + `rustls`).
//!
//! One GET with a caller timeout and a body cap. No retries, no
//! redirects policy beyond `ureq` defaults, no robots handling —
//! Week-3 crawl work. Fetch failures are data for an F1
//! `task(kind='validate_fact')`, never silent skips (see `xtask fetch-url`).

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
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut response = agent.get(url).call().map_err(|e| map_ureq_error(e, url))?;
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
}
