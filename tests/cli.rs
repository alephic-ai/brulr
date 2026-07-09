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

#[test]
fn missing_grok_harness_errors() {
    let out = burn_with_missing_harness("grok");
    assert!(!out.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("can't find the `grok` harness"), "stderr was: {stderr}");
}

/// Validation runs before spawning the harness, so PATH can stay empty.
fn burn_args(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_brulr"))
        .args(args)
        .env("PATH", "")
        .output()
        .expect("failed to run brulr")
}

#[test]
fn model_on_wrong_harness_errors() {
    // Default harness is claude; grok-4.5 is a known grok model.
    let out = burn_args(&["burn", "100", "--model", "grok-4.5"]);
    assert!(!out.status.success(), "expected non-zero exit");
    assert_eq!(out.status.code(), Some(2), "expected usage exit 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("model 'grok-4.5' is for harness 'grok'"),
        "stderr was: {stderr}"
    );
    assert!(stderr.contains("--harness grok"), "stderr was: {stderr}");
}

#[test]
fn effort_on_model_without_effort_errors() {
    let out = burn_args(&[
        "burn",
        "100",
        "--harness",
        "grok",
        "--model",
        "grok-composer-2.5-fast",
        "--effort",
        "high",
    ]);
    assert!(!out.status.success(), "expected non-zero exit");
    assert_eq!(out.status.code(), Some(2), "expected usage exit 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("does not support --effort"),
        "stderr was: {stderr}"
    );
}
