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
fn max_records_caps_each_draft_and_refuses_out_of_range() {
    let dir = tempfile::tempdir().unwrap();
    let store = init_store(&dir);
    let id = seed_capture(&store, "usa-wa-sos", true);
    let analyze = |extra: &[&str]| {
        let mut args = vec!["analyze", "--store", &store, "--slug", "usa-wa-sos"];
        args.extend_from_slice(extra);
        xtask().args(&args).output().unwrap()
    };
    // Out of range: exit 2 before any draft (no facts, watermark unmoved).
    for bad in ["0", "33"] {
        let out = analyze(&["--scan-captures", "--max-records", bad]);
        assert_eq!(out.status.code(), Some(2), "{bad}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("--max-records"), "got: {stderr}");
    }
    let conn = rusqlite::Connection::open(format!("{store}/handbook.sqlite")).unwrap();
    let facts = || -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM fact WHERE capture_id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(facts(), 0);
    assert_eq!(
        handbook_store::db::get_capture_watermark(&conn, "usa-wa-sos").unwrap(),
        0
    );
    // The `=` form is refused the same way, never read as the default.
    let out = analyze(&["--scan-captures", "--max-records=33"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(facts(), 0);
    // The stub drafts two records per call; a budget of one keeps one,
    // and the run prints the budget it drafted the slug with.
    let out = analyze(&["--scan-captures", "--max-records", "1"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("max_records 1 for usa-wa-sos") && stdout.contains("max_records=1"),
        "got: {stdout}"
    );
    assert_eq!(facts(), 1);
    // Hand-fed evidence takes the same budget.
    let out = analyze(&["--input", TEXT, "--max-records", "1"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success()
            && stdout.contains("max_records 1 for usa-wa-sos")
            && stdout.contains("proposed: 1"),
        "got: {stdout}"
    );
}

#[test]
fn max_records_equals_form_caps_the_same_and_the_default_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let store = init_store(&dir);
    let id = seed_capture(&store, "usa-or-sos", true);
    let analyze = |extra: &[&str]| {
        let mut args = vec!["analyze", "--store", &store, "--slug", "usa-or-sos"];
        args.extend_from_slice(extra);
        xtask().args(&args).output().unwrap()
    };
    let out = analyze(&["--scan-captures", "--max-records=1"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("max_records 1 for usa-or-sos"),
        "got: {stdout} {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let conn = rusqlite::Connection::open(format!("{store}/handbook.sqlite")).unwrap();
    let facts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM fact WHERE capture_id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(facts, 1, "the = form keeps one of the stub's two records");
    // Without the flag the run names the default budget.
    let out = analyze(&["--scan-captures"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success()
            && stdout.contains("max_records 8 for usa-or-sos (default)")
            && stdout.contains("max_records=8"),
        "got: {stdout}"
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
