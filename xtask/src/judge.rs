//! `cargo xtask judge`: the one command a hackathon judge runs on a
//! fresh clone (macOS or Linux, Rust + Node) to get a working local
//! copy — public site on http://127.0.0.1:18318/apostille, admin on
//! http://127.0.0.1:18319 — with no keys, synthetic data and recorded
//! Meta model responses (`--live` drafts through the judge's own
//! `META_API_KEY` instead).
//!
//! Owner: lane SA-29 judge-kit (`docs/lanes/sa-29.md`). B0 stub: a
//! fresh `up` boot; the lane adds the prerequisite doctor, seeded
//! accepted dossiers, the local staff sign-in link and the summary.

/// Entry point (the builder wires the `judge` arm in `main.rs`).
pub fn run(args: &[String]) {
    let mut forwarded: Vec<String> = vec!["--fresh".to_string()];
    forwarded.extend(args.iter().cloned());
    crate::up::run(&forwarded);
}
