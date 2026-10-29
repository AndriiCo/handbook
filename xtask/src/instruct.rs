//! `xtask instruct-sweep` + `xtask instruct-demo` (spec
//! `docs/specs/instruction-worker.md`).
//!
//! The sweep runs server-side on a timer: new email / comment /
//! transcript captures become instruction drafts + proposed fact
//! updates (human review still decides). The demo is a frozen
//! Washington-state drill on a temp store.

use std::path::PathBuf;

use atlas::model::StubPort;
use handbook_store::db;
use handbook_store::store::Store;
use instruct::fixture::{WA_SLUG, seed_wa_demo, wa_script};
use instruct::template::TemplatePack;

/// `instruct-sweep [--store …] [--pack …] [--slug …] [--model …]
/// [--actor …] [--live] [--dry-run]`.
pub fn run_sweep(args: &[String]) {
    let pack_path = super::flag(args, "pack")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("crates/instruct/packs/general.toml"));
    let pack = TemplatePack::load(&pack_path).unwrap_or_else(|e| {
        eprintln!("instruct-sweep: {e}");
        std::process::exit(2);
    });
    let model = super::flag(args, "model").unwrap_or_else(|| pack.worker.model.clone());
    let actor = super::flag(args, "actor").unwrap_or_else(|| "instruct-sweep".to_string());
    let slugs: Option<Vec<String>> = super::flag(args, "slug").map(|s| vec![s]);
    let dry = args.iter().any(|a| a == "--dry-run");
    let live = args.iter().any(|a| a == "--live");
    let store = super::open_store(args);
    let conn = super::open_db(&store);
    let now = super::unix_now();
    if live {
        let client = super::atlas::live_client(args);
        let outcomes = instruct::worker::sweep_all(
            &conn,
            store.root(),
            &client,
            &pack,
            &model,
            &actor,
            now,
            dry,
            slugs.as_deref(),
        )
        .unwrap_or_else(|e| {
            eprintln!("instruct-sweep: {e}");
            std::process::exit(1);
        });
        report(&outcomes);
    } else {
        let port = StubPort::new("{}", "{}");
        let outcomes = instruct::worker::sweep_all(
            &conn,
            store.root(),
            &port,
            &pack,
            &model,
            &actor,
            now,
            dry,
            slugs.as_deref(),
        )
        .unwrap_or_else(|e| {
            eprintln!("instruct-sweep: {e}");
            std::process::exit(1);
        });
        report(&outcomes);
    }
}

fn report(outcomes: &[instruct::worker::AuthorityOutcome]) {
    if outcomes.is_empty() {
        println!("instruct-sweep: no triggers");
        return;
    }
    for outcome in outcomes {
        println!(
            "{}: triggers={} proposed={} suppressed={} identical={} task={:?} errors={}",
            outcome.slug,
            outcome.triggers,
            outcome.report.proposed,
            outcome.report.suppressed,
            outcome.report.identical,
            outcome.report.task_id,
            outcome.report.errors.len()
        );
        for error in &outcome.report.errors {
            println!("  error: {error}");
        }
    }
}

/// `xtask instruct-demo`: frozen WA drill on a temp store (never
/// prod — there is no --store flag to smuggle one in).
pub fn run_demo(args: &[String]) {
    if !args.is_empty() {
        eprintln!("instruct-demo takes no flags (frozen drill on a temp store)");
        std::process::exit(2);
    }
    let tmp = tempfile::tempdir().unwrap_or_else(|e| {
        eprintln!("instruct-demo: temp store: {e}");
        std::process::exit(1);
    });
    let root = tmp.path().join("store");
    std::fs::create_dir_all(&root).unwrap_or_else(|e| {
        eprintln!("instruct-demo: mkdir: {e}");
        std::process::exit(1);
    });
    let store = Store::new(&root);
    let conn = db::open(&store).unwrap_or_else(|e| {
        eprintln!("instruct-demo: open: {e}");
        std::process::exit(1);
    });
    db::migrate(&conn).unwrap_or_else(|e| {
        eprintln!("instruct-demo: migrate: {e}");
        std::process::exit(1);
    });
    let demo = seed_wa_demo(&conn, &root, 1_700_000_000).unwrap_or_else(|e| {
        eprintln!("instruct-demo: seed: {e}");
        std::process::exit(1);
    });
    let pack = TemplatePack::load(&PathBuf::from("crates/instruct/packs/general.toml"))
        .unwrap_or_else(|e| {
            eprintln!("instruct-demo: {e}");
            std::process::exit(2);
        });
    let port = atlas::model::StubPort::with_script(wa_script(&demo.capture_ids));
    let outcome = instruct::worker::run_authority(
        &conn,
        &root,
        &port,
        &pack,
        "spark",
        WA_SLUG,
        "instruct-demo",
        1_700_000_001,
        false,
    )
    .unwrap_or_else(|e| {
        eprintln!("instruct-demo: {e}");
        std::process::exit(1);
    });
    println!(
        "=== instructions ({WA_SLUG}) ===\n{}",
        outcome.instruction_md
    );
    let mut stmt = conn
        .prepare(
            "SELECT topic, statement, reasoning FROM fact WHERE status = 'proposed' ORDER BY id",
        )
        .unwrap();
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    println!("=== proposed facts ({}) ===", rows.len());
    for (topic, statement, reasoning) in &rows {
        println!("--- [{topic}] {statement}\n{reasoning}\n");
    }
    println!(
        "demo: triggers={} proposed={} task={:?} (temp store {}, prod untouched)",
        outcome.triggers,
        outcome.report.proposed,
        outcome.report.task_id,
        root.display()
    );
}
