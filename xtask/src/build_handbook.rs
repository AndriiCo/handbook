//! Approved-changes publish CLI (F11): thin wrapper over `publish`.
//!
//! `xtask build-handbook [--store <path>] [--out <dir>]
//! [--base-url <url>] [--actor <name>]`: accepted-only pages, atomic
//! swap, `build` + `audit` rows, sitemap/robots merge.

/// CLI entry: build and print the report.
pub fn run(args: &[String]) {
    let store = crate::open_store(args);
    let conn = crate::open_db(&store);
    if let Err(e) = handbook_store::db::migrate(&conn) {
        eprintln!("migrate: {e}");
        std::process::exit(1);
    }
    let out = crate::flag(args, "out").unwrap_or_else(|| "site".to_string());
    let base_url =
        crate::flag(args, "base-url").unwrap_or_else(|| "https://andrii.ai/apostille".to_string());
    let actor = crate::flag(args, "actor").unwrap_or_else(|| "xtask".to_string());
    let report = publish::build_handbook(
        &conn,
        &store,
        &publish::BuildOptions {
            out_dir: out.into(),
            base_url,
            actor,
            at: crate::unix_now(),
        },
    )
    .unwrap_or_else(|e| {
        eprintln!("build-handbook: {e}");
        std::process::exit(1);
    });
    println!(
        "build_no={} facts_hash={} pages={} conflicts={} dir={}",
        report.build_no,
        report.facts_hash,
        report.pages,
        report.conflicts,
        report.dir.display(),
    );
}
