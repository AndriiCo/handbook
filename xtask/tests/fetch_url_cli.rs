//! End-to-end `xtask fetch-url` contract: capture, dedup, failure task.
//!
//! Drives the real binary against a temp store and a loopback HTTP
//! server on port 0. No network beyond 127.0.0.1.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

fn xtask() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
}

fn init_store(dir: &tempfile::TempDir) -> String {
    let store = dir.path().join("store").display().to_string();
    let status = xtask()
        .args(["init", "--store", &store])
        .status()
        .expect("run init");
    assert!(status.success(), "init failed: {status}");
    let status = xtask()
        .args([
            "add-authority",
            "--store",
            &store,
            "--kind",
            "state",
            "--country",
            "USA",
            "--region",
            "Washington",
            "--name",
            "WA Secretary of State",
            "--slug",
            "usa-wa-sos",
        ])
        .status()
        .expect("run add-authority");
    assert!(status.success(), "add-authority failed: {status}");
    store
}

/// Serve `replies` canned HTTP/1.0 replies on loopback port 0, then exit.
/// Returns the base URL and the server thread handle.
fn serve_many(
    replies: Vec<(&'static str, &'static [u8])>,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let url = format!(
        "http://127.0.0.1:{}/page",
        listener.local_addr().unwrap().port()
    );
    let handle = std::thread::spawn(move || {
        for (status_line, body) in replies {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let head = format!(
                "{status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).expect("write head");
            stream.write_all(body).expect("write body");
        }
    });
    (url, handle)
}

fn fetch(store: &str, url: &str, extra: &[&str]) -> std::process::Output {
    let mut cmd = xtask();
    cmd.args([
        "fetch-url",
        "--store",
        store,
        "--slug",
        "usa-wa-sos",
        "--url",
        url,
    ]);
    for flag in extra {
        cmd.arg(flag);
    }
    cmd.output().expect("run fetch-url")
}

#[test]
fn capture_then_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let store = init_store(&dir);
    let (url, server) = serve_many(vec![
        ("HTTP/1.0 200 OK", b"<html><body>PO Box 40220</body></html>"),
        ("HTTP/1.0 200 OK", b"<html><body>PO Box 40220</body></html>"),
    ]);

    let first = fetch(&store, &url, &[]);
    assert!(
        first.status.success(),
        "first fetch failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let out = String::from_utf8_lossy(&first.stdout);
    assert!(out.starts_with("capture: "), "got: {out}");

    let second = fetch(&store, &url, &[]);
    assert!(
        second.status.success(),
        "second fetch failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let out = String::from_utf8_lossy(&second.stdout);
    assert!(out.starts_with("duplicate: "), "got: {out}");

    // Exactly one watch capture: the re-fetch created no new work.
    // (Migration 005 seeds an id=0 sentinel row, so count warc rows.)
    let db = format!("{store}/handbook.sqlite");
    let conn = rusqlite::Connection::open(db).unwrap();
    let captures: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM capture WHERE kind = 'warc'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(captures, 1);
    server.join().unwrap();
}

#[test]
fn dead_url_files_validate_task_and_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    let store = init_store(&dir);
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let dead = format!("http://127.0.0.1:{port}/gone");
    let out = fetch(&store, &dead, &[]);
    assert!(!out.status.success(), "dead fetch must exit nonzero");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("task: fetch failure recorded"),
        "got: {stdout}"
    );

    let db = format!("{store}/handbook.sqlite");
    let conn = rusqlite::Connection::open(db).unwrap();
    let tasks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM task WHERE kind = 'validate_fact'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tasks, 1);

    // A second dead fetch refreshes the same row instead of filing another.
    let out = fetch(&store, &dead, &[]);
    assert!(!out.status.success());
    let tasks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM task WHERE kind = 'validate_fact'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tasks, 1);
}

#[test]
fn robots_disallow_skips_without_error() {
    let dir = tempfile::tempdir().unwrap();
    let store = init_store(&dir);
    // Third-party fetch requests /robots.txt first; disallow-all skips
    // the page fetch, so the server only sees one request.
    let (url, server) = serve_many(vec![("HTTP/1.0 200 OK", b"User-agent: *\nDisallow: /")]);

    let out = fetch(&store, &url, &["--source-kind", "website_thirdparty"]);
    assert!(
        out.status.success(),
        "robots skip must exit 0: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("skipped (robots disallow)"),
        "got: {stdout}"
    );

    let db = format!("{store}/handbook.sqlite");
    let conn = rusqlite::Connection::open(db).unwrap();
    let captures: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM capture WHERE kind = 'warc'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(captures, 0);
    let robots: String = conn
        .query_row("SELECT robots FROM source", [], |row| row.get(0))
        .unwrap();
    assert!(robots.starts_with("disallow-all"), "got: {robots:?}");
    server.join().unwrap();
}
