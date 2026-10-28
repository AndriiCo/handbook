//! `xtask demo-vietnam` contract: frozen drill, temp store, PASS report.
use std::process::Command;

#[test]
fn drill_passes_and_touches_no_prod_store() {
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("demo-vietnam")
        .output()
        .expect("run demo-vietnam");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    for step in [
        "[1/5]",
        "[2/5]",
        "[3/5]",
        "[4/5]",
        "[5/5]",
        "demo-vietnam: 5/5 PASS",
    ] {
        assert!(stdout.contains(step), "missing {step}: {stdout}");
    }
    assert!(!stdout.contains("FAIL"), "{stdout}");
    // No store dir created in the working tree by the drill.
    assert!(!std::path::Path::new("store/handbook.sqlite").exists());
}

#[test]
fn drill_rejects_flags() {
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["demo-vietnam", "--store", "store"])
        .output()
        .expect("run demo-vietnam");
    assert!(!out.status.success(), "flags must be rejected");
}
