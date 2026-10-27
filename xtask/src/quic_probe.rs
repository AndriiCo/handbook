//! `quic-probe`: minimal HTTP/3 GET client (operational smoke tool).
//!
//! Dials a UDP addr with SNI + CA verification, sends one GET, prints
//! status + `alt-svc` + body. No `--insecure` backdoor: staging uses
//! `--cacert` (staging CA PEM), prod verifies against webpki roots.
//!
//! ```sh
//! cargo xtask quic-probe --addr 127.0.0.1:3443 \
//!   --server-name admin.andrii.ai --cacert tls-staging/ca.pem \
//!   --path /apostille/healthz [--host admin.andrii.ai]
//! ```

use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Buf as _;

/// `quic-probe --addr <ip:port> --server-name <sni> [--cacert <pem>]
/// [--path <p>] [--host <h>]`.
pub fn run(args: &[String]) {
    let addr: SocketAddr = crate::required(args, "addr")
        .parse()
        .unwrap_or_else(|_| fail("invalid --addr (want ip:port, no DNS)"));
    let server_name = crate::required(args, "server-name");
    let path = crate::flag(args, "path").unwrap_or_else(|| "/apostille/healthz".to_string());
    let host = crate::flag(args, "host").unwrap_or_else(|| server_name.clone());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| fail(&format!("tokio runtime: {e}")));
    rt.block_on(async {
        match probe(&addr, &server_name, &host, &path, cacert(args)).await {
            Ok(()) => {}
            Err(e) => fail(&e),
        }
    });
}

fn cacert(args: &[String]) -> Option<Vec<u8>> {
    crate::flag(args, "cacert")
        .map(|path| std::fs::read(&path).unwrap_or_else(|_| fail(&format!("read --cacert {path}"))))
}

fn fail(message: &str) -> ! {
    eprintln!("quic-probe: {message}");
    std::process::exit(1);
}

async fn probe(
    addr: &SocketAddr,
    server_name: &str,
    host: &str,
    path: &str,
    cacert_pem: Option<Vec<u8>>,
) -> Result<(), String> {
    let mut roots =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(pem) = cacert_pem.as_deref() {
        let mut cursor = std::io::Cursor::new(pem);
        for der in rustls_pemfile::certs(&mut cursor) {
            let der = der.map_err(|e| format!("parse --cacert PEM: {e}"))?;
            roots
                .add(der)
                .map_err(|e| format!("add --cacert root: {e:?}"))?;
        }
    }
    let mut rustls_config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    rustls_config.alpn_protocols = vec![b"h3".to_vec()];
    let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(rustls_config)
        .map_err(|e| format!("QUIC client crypto: {e}"))?;
    let mut client_config = quinn::ClientConfig::new(Arc::new(quic_crypto));
    client_config.transport_config(Arc::new({
        let mut t = quinn::TransportConfig::default();
        t.max_idle_timeout(Some(std::time::Duration::from_secs(10).try_into().unwrap()));
        t
    }));
    let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap())
        .map_err(|e| format!("QUIC endpoint bind: {e}"))?;
    endpoint.set_default_client_config(client_config);
    let connecting = endpoint
        .connect(*addr, server_name)
        .map_err(|e| format!("QUIC connect to {addr}: {e}"))?;
    let conn = connecting
        .await
        .map_err(|e| format!("QUIC handshake with {addr}: {e}"))?;
    println!("quic-probe: connected to {addr} (SNI {server_name})");
    let h3_conn = h3_quinn::Connection::new(conn);
    let (mut driver, mut sender) = h3::client::new(h3_conn)
        .await
        .map_err(|e| format!("h3 client: {e}"))?;
    let drive =
        tokio::spawn(
            async move { futures_util::future::poll_fn(|cx| driver.poll_close(cx)).await },
        );
    let uri: http::Uri = format!("https://{host}{path}")
        .parse()
        .map_err(|e| format!("bad --path: {e}"))?;
    let req = http::Request::builder()
        .uri(uri)
        .header("host", host)
        .body(())
        .map_err(|e| format!("build request: {e}"))?;
    let mut stream = sender
        .send_request(req)
        .await
        .map_err(|e| format!("h3 send: {e}"))?;
    // GET has no body: half-close so the server answers (else it
    // waits for END_STREAM and the request times out).
    stream
        .finish()
        .await
        .map_err(|e| format!("h3 finish: {e}"))?;
    let resp = stream
        .recv_response()
        .await
        .map_err(|e| format!("h3 response: {e}"))?;
    println!("quic-probe: status {}", resp.status());
    if let Some(alt) = resp.headers().get("alt-svc") {
        println!("quic-probe: alt-svc {}", alt.to_str().unwrap_or("<?>"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = stream
        .recv_data()
        .await
        .map_err(|e| format!("h3 data: {e}"))?
    {
        body.extend_from_slice(chunk.chunk());
        if body.len() > 1 << 20 {
            return Err("body exceeds 1 MiB cap".to_string());
        }
    }
    println!("quic-probe: body {}", String::from_utf8_lossy(&body));
    drop(drive);
    endpoint.wait_idle().await;
    Ok(())
}
