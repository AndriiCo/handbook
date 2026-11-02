//! `xtask analyze --scan-captures`: Option-B watermark scan.
//!
//! Drives the real binary against a temp store with seeded captures
//! (sidecar present/missing, pre-watermarked ids, unsupported-lang skip).

use std::process::Command;

fn xtask() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
}

// StubClient-compatible (bytes 0..12, 13..24).
const TEXT: &str = "PO Box 40220, Mon-Fri 9-5!";

fn init_store(dir: &tempfile::TempDir) -> String {
    let store = dir.path().join("store").display().to_string();
    assert!(
        xtask()
            .args(["init", "--store", &store])
            .status()
            .unwrap()
            .success()
    );
    for (slug, name) in [
        ("usa-wa-sos", "WA Secretary of State"),
        ("usa-or-sos", "OR Secretary of State"),
    ] {
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
                    "Northwest",
                    "--name",
                    name,
                    "--slug",
                    slug,
                ])
                .status()
                .unwrap()
                .success()
        );
    }
    store
}

fn seed_capture(store: &str, slug: &str, with_sidecar: bool) -> i64 {
    let conn = rusqlite::Connection::open(format!("{store}/handbook.sqlite")).unwrap();
    let authority: i64 = conn
        .query_row("SELECT id FROM authority WHERE slug = ?1", [slug], |r| {
            r.get(0)
        })
        .unwrap();
    conn.execute(
        "INSERT INTO capture(authority_id, captured_at, kind, path, bytes_raw, bytes_stored, sha256, trust)
         VALUES (?1, 1_800_000_050, 'warc', 'blobs/synthetic', 4, 4, 'x', 'official')",
        [authority],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    if with_sidecar {
        let dir = std::path::Path::new(store).join("text");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{id}.txt")), TEXT).unwrap();
    }
    id
}

#[test]
fn scan_analyzes_new_skips_missing_and_respects_watermark() {
    let dir = tempfile::tempdir().unwrap();
    let store = init_store(&dir);
    let a = seed_capture(&store, "usa-wa-sos", true);
    let _b = seed_capture(&store, "usa-wa-sos", false); // missing sidecar
    let c = seed_capture(&store, "usa-wa-sos", true);
    // Pre-watermark past `a`: the scan must only touch b and c.
    let conn = rusqlite::Connection::open(format!("{store}/handbook.sqlite")).unwrap();
    conn.execute(
        "INSERT INTO watermark(authority_slug, last_run_at, last_prompt_version, last_capture_id)
         VALUES ('usa-wa-sos', 1_800_000_000, 'aa-prompt-v1', ?1)",
        [a],
    )
    .unwrap();
    drop(conn);
    let out = xtask()
        .args([
            "analyze",
            "--store",
            &store,
            "--slug",
            "usa-wa-sos",
            "--scan-captures",
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
        stdout.contains("scanned=2") && stdout.contains("missing sidecar"),
        "got: {stdout}"
    );
    let conn = rusqlite::Connection::open(format!("{store}/handbook.sqlite")).unwrap();
    let facts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM fact WHERE capture_id = ?1",
            [c],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(facts, 2); // stub emits 2 records for capture c only
    let stale: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM fact WHERE capture_id = ?1",
            [a],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stale, 0);
    assert_eq!(
        handbook_store::db::get_capture_watermark(&conn, "usa-wa-sos").unwrap(),
        c
    );
    // Re-run is a no-op (idempotent).
    let out = xtask()
        .args([
            "analyze",
            "--store",
            &store,
            "--slug",
            "usa-wa-sos",
            "--scan-captures",
        ])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("scanned=0"),
        "got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn scan_with_unsupported_lang_skips_without_facts() {
    let dir = tempfile::tempdir().unwrap();
    let store = init_store(&dir);
    let id = seed_capture(&store, "usa-or-sos", true);
    let out = xtask()
        .args([
            "analyze",
            "--store",
            &store,
            "--slug",
            "usa-or-sos",
            "--scan-captures",
            "--lang",
            "de",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let conn = rusqlite::Connection::open(format!("{store}/handbook.sqlite")).unwrap();
    let facts: i64 = conn
        .query_row("SELECT COUNT(*) FROM fact", [], |r| r.get(0))
        .unwrap();
    assert_eq!(facts, 0);
    assert_eq!(
        handbook_store::db::get_capture_watermark(&conn, "usa-or-sos").unwrap(),
        id
    );
}
