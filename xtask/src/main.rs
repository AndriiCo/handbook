//! Project task runner. Use `cargo xtask <task>` instead of shell scripts.
//!
//! Keeping tasks in Rust reuses the project's language, libraries, and
//! error handling instead of scattering logic across shell dialects.

use std::process::Command;

fn main() {
    let task = std::env::args().nth(1);
    let status = match task.as_deref() {
        None | Some("--help" | "-h" | "help") => return print_help(),
        Some("ci") => run_ci(),
        Some(unknown) => {
            eprintln!("unknown task: {unknown}\n");
            print_help();
            std::process::exit(2);
        }
    };
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn print_help() {
    println!(
        "Usage: cargo xtask <task>\n\nTasks:\n  ci    fmt --check, clippy, and tests (what CI runs)"
    );
}

fn run_ci() -> std::process::ExitStatus {
    for args in [
        ["fmt", "--all", "--check"].as_slice(),
        [
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ]
        .as_slice(),
        ["test", "--workspace"].as_slice(),
    ] {
        let status = Command::new("cargo")
            .args(args)
            .status()
            .expect("failed to run cargo");
        if !status.success() {
            return status;
        }
    }
    Command::new("true").status().expect("failed to run true")
}
