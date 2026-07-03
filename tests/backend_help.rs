//! Nightly CLI flag-drift smoke test.
//!
//! For each backend, gets its `command()` and `required_help_flags()`, runs
//! `<command> --help` (and `<command> exec --help` for Codex, which wraps a
//! subcommand), and asserts that every flag string appears somewhere in the
//! help output. If a CLI is not on PATH the backend is skipped with a message —
//! a maintainer without all four CLIs installed gets no false failure.
//!
//! All tests here are `#[ignore]`d so that `cargo test` and CI skip them.
//! The nightly `.github/workflows/backend-drift.yml` runs them with
//! `cargo test --test backend_help -- --ignored --nocapture` after installing
//! each CLI.

use std::process::Command;

use alix::{
    backend::backend_for,
    config::{AskConfig, BackendKind},
};

/// Returns true when `command` resolves on PATH.
fn is_installed(command: &str) -> bool {
    // `which` is POSIX; on Windows this would need `where`. The workflow runs
    // on ubuntu-latest so this is sufficient.
    Command::new("which")
        .arg(command)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Runs `argv[0] argv[1..]` and returns the combined stdout+stderr so flag
/// strings that appear only in the error stream are still matched. `--help`
/// often exits non-zero on some CLIs; we ignore the exit code.
fn help_text(argv: &[&str]) -> String {
    let out = Command::new(argv[0])
        .args(&argv[1..])
        .output()
        .unwrap_or_else(|e| panic!("failed to run {:?}: {e}", argv));
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

/// Constructs a default `AskConfig` wired to `kind`.
fn cfg(kind: BackendKind) -> AskConfig {
    AskConfig {
        backend: kind,
        ..AskConfig::default()
    }
}

/// Checks that every `required_help_flags()` string appears in the help output.
/// Returns the list of missing flags (empty = pass).
fn missing_flags(backend_name: &str, help: &str, flags: &[&str]) -> Vec<String> {
    let mut missing = Vec::new();
    for &flag in flags {
        if !help.contains(flag) {
            eprintln!("  MISSING in {backend_name} --help: {flag:?}");
            missing.push(flag.to_string());
        }
    }
    missing
}

#[test]
#[ignore]
fn backend_claude_help_contains_required_flags() {
    let backend = backend_for(&cfg(BackendKind::Claude)).unwrap();
    let cmd = backend.command();

    if !is_installed(cmd) {
        eprintln!("skipping claude: {cmd} not installed");
        return;
    }

    let help = help_text(&[cmd, "--help"]);
    let missing = missing_flags("claude", &help, backend.required_help_flags());
    assert!(
        missing.is_empty(),
        "claude --help is missing required flags: {missing:?}\n\
         — these flags may have been renamed or removed in a CLI update"
    );
}

#[test]
#[ignore]
fn backend_gemini_help_contains_required_flags() {
    let backend = backend_for(&cfg(BackendKind::Gemini)).unwrap();
    let cmd = backend.command();

    if !is_installed(cmd) {
        eprintln!("skipping gemini: {cmd} not installed");
        return;
    }

    let help = help_text(&[cmd, "--help"]);
    let missing = missing_flags("gemini", &help, backend.required_help_flags());
    assert!(
        missing.is_empty(),
        "gemini --help is missing required flags: {missing:?}\n\
         — these flags may have been renamed or removed in a CLI update"
    );
}

#[test]
#[ignore]
fn backend_codex_help_contains_required_flags() {
    let backend = backend_for(&cfg(BackendKind::Codex)).unwrap();
    let cmd = backend.command();

    if !is_installed(cmd) {
        eprintln!("skipping codex: {cmd} not installed");
        return;
    }

    // Codex uses a subcommand (`codex exec …`), so some flags only appear in
    // `codex exec --help`. Check the top-level help first (for global flags like
    // `--ask-for-approval`), then the subcommand help (for `--sandbox` and the
    // `exec` subcommand name itself).
    let top_help = help_text(&[cmd, "--help"]);
    let exec_help = help_text(&[cmd, "exec", "--help"]);
    let combined = format!("{top_help}\n{exec_help}");

    let missing = missing_flags("codex", &combined, backend.required_help_flags());
    assert!(
        missing.is_empty(),
        "codex --help / codex exec --help is missing required flags: {missing:?}\n\
         — these flags may have been renamed or removed in a CLI update"
    );
}

#[test]
#[ignore]
fn backend_copilot_help_contains_required_flags() {
    let backend = backend_for(&cfg(BackendKind::Copilot)).unwrap();
    let cmd = backend.command();

    if !is_installed(cmd) {
        eprintln!("skipping copilot: {cmd} not installed");
        return;
    }

    let help = help_text(&[cmd, "--help"]);
    let missing = missing_flags("copilot", &help, backend.required_help_flags());
    assert!(
        missing.is_empty(),
        "copilot --help is missing required flags: {missing:?}\n\
         — these flags may have been renamed or removed in a CLI update"
    );
}
