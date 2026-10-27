//! Contradiction review (F6): print every conflict with citations.
//!
//! `xtask review-conflicts [--store <path>]`: same authority + same
//! topic, two `accepted` facts with different statements (query owned
//! by the `publish` crate, which also renders the page banners).
//! Conflicts never auto-resolve; resolving one is accept/supersede +
//! audit, done through the approval path.

/// CLI entry: print conflicts, exit 0 (review output, not a gate).
pub fn run(args: &[String]) {
    let store = crate::open_store(args);
    let conn = crate::open_db(&store);
    let found = publish::conflicts(&conn).unwrap_or_else(|e| {
        eprintln!("review-conflicts: {e}");
        std::process::exit(1);
    });
    for c in &found {
        println!(
            "CONFLICT {} topic={} fact#{} {:?} (capture #{}) vs fact#{} {:?} (capture #{})",
            c.authority_slug,
            c.topic,
            c.first_id,
            c.first_statement,
            c.first_capture_id,
            c.second_id,
            c.second_statement,
            c.second_capture_id,
        );
    }
    println!("conflicts: {}", found.len());
}
