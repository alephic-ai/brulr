//! End-to-end CLI tests that drive the built `brulr` binary.

use std::process::Command;

/// Run `brulr burn 100 --harness <harness>` with an emptied PATH, so the
/// harness binary can't be resolved. brülr is invoked by absolute path, so it
/// still starts; only the inner `Command::new("claude"|"codex")` fails. The
/// real harness never runs, so no tokens are spent.
fn burn_with_missing_harness(harness: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_brulr"))
        .args(["burn", "100", "--harness", harness])
        .env("PATH", "")
        .output()
        .expect("failed to run brulr")
}

#[test]
fn missing_claude_harness_errors() {
    let out = burn_with_missing_harness("claude");
    assert!(!out.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("can't find the `claude` harness"), "stderr was: {stderr}");
}

#[test]
fn missing_codex_harness_errors() {
    let out = burn_with_missing_harness("codex");
    assert!(!out.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("can't find the `codex` harness"), "stderr was: {stderr}");
}
