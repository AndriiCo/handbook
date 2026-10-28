//! `xtask fetch-slug`: sweep creates, dedups, and reports failures.
//!
//! Drives the real binary against a temp store and loopback HTTP
//! servers on port 0. No network beyond 127.0.0.1.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

fn xtask() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
}

fn init_store(dir: &tempfile::TempDir) -> String {
    let store = dir.path().join("store").display().to_string();
    assert!(
        xtask()
            .args(["init", "--store", &store])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        xtask()
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
            .unwrap()
            .success()
    );
    store
}

fn serve_many(
    replies: Vec<(&'static str, &'static [u8])>,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let base = format!(
        "http://127.0.0.1:{}/",
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
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
        }
    });
    (base, handle)
}

fn seed_sources(store: &str, urls: &[String]) {
    let conn = rusqlite::Connection::open(format!("{store}/handbook.sqlite")).unwrap();
    let authority: i64 = conn
        .query_row(
            "SELECT id FROM authority WHERE slug = 'usa-wa-sos'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for url in urls {
        conn.execute(
            "INSERT INTO source(authority_id, kind, uri, notes)
             VALUES (?1, 'website_official', ?2, 'cadence:daily;')",
            rusqlite::params![authority, url],
        )
        .unwrap();
    }
}

#[test]
fn sweep_creates_then_dedups() {
    let dir = tempfile::tempdir().unwrap();
    let store = init_store(&dir);
    let (base, server) = serve_many(vec![
        ("HTTP/1.0 200 OK", b"<p>page one</p>"),
        ("HTTP/1.0 200 OK", b"<p>page two</p>"),
        ("HTTP/1.0 200 OK", b"<p>page one</p>"),
        ("HTTP/1.0 200 OK", b"<p>page two</p>"),
    ]);
    seed_sources(&store, &[format!("{base}a"), format!("{base}b")]);
    let out = xtask()
        .args([
            "fetch-slug",
            "--store",
            &store,
            "--slug",
            "usa-wa-sos",
            "--actor",
            "t",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("fetched=") && stdout.contains("urls=2"),
        "got: {stdout}"
    );
    let out = xtask()
        .args(["fetch-slug", "--store", &store, "--slug", "usa-wa-sos"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("dedup=2"), "got: {stdout}");
    let conn = rusqlite::Connection::open(format!("{store}/handbook.sqlite")).unwrap();
    let captures: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM capture WHERE kind = 'warc'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(captures, 2);
    server.join().unwrap();
}

#[test]
fn dead_url_reports_failed_files_task_and_limits() {
    let dir = tempfile::tempdir().unwrap();
    let store = init_store(&dir);
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    // No live server: --limit 1 touches only the dead URL (a serving
    // thread would block on accept forever).
    seed_sources(
        &store,
        &[
            format!("http://127.0.0.1:{port}/gone"),
            "http://127.0.0.1:9/unused".to_string(),
        ],
    );
    // Sweep still exits 0 (failure is a task, not an abort).
    let out = xtask()
        .args([
            "fetch-slug",
            "--store",
            &store,
            "--slug",
            "usa-wa-sos",
            "--limit",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("failed=1") && stdout.contains("failed: http"),
        "got: {stdout}"
    );
    let conn = rusqlite::Connection::open(format!("{store}/handbook.sqlite")).unwrap();
    let tasks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM task WHERE kind = 'validate_fact' AND title LIKE 'fetch failed: %'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tasks, 1);
}
