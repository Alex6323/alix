//! End-to-end CLI integration tests: each runs the built `alix` binary as a
//! subprocess against temp decks and a temp progress store, asserting on exit
//! status and output. Unlike `tests/eval.rs` these are fully deterministic — no
//! real Claude — so they run in CI on every `make check`.
//!
//! A recurring property here is that a damaged progress store fails *safely*:
//! the command errors and the file on disk is left exactly as it was, never
//! silently overwritten with an empty store.

use std::{
    path::Path,
    process::{Command, Output},
};

use tempfile::TempDir;

/// Runs `alix <args...>` and returns its captured output.
fn alix(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_alix"))
        .args(args)
        .output()
        .expect("failed to run the alix binary")
}

/// Writes `contents` to `dir/name` and returns its path as a string.
fn write(dir: &Path, name: &str, contents: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path.to_str().unwrap().to_string()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

const VALID_DECK: &str = "# What is 2 + 2?\n    4\n";

#[test]
fn check_accepts_a_valid_deck() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.txt", VALID_DECK);
    let out = alix(&["check", &deck]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("1 cards"), "stdout: {}", stdout(&out));
}

#[test]
fn check_rejects_a_malformed_deck() {
    let dir = TempDir::new().unwrap();
    // A card front with no answer line is a parse error.
    let deck = write(dir.path(), "broken.txt", "# a front with no answer\n");
    let out = alix(&["check", &deck]);
    assert!(
        !out.status.success(),
        "a malformed deck should fail the check"
    );
    assert!(stderr(&out).contains("error:"), "stderr: {}", stderr(&out));
}

#[test]
fn stats_reports_a_fresh_deck_against_an_empty_store() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.txt", VALID_DECK);
    let store = dir.path().join("progress.json"); // does not exist yet
    let out = alix(&["stats", &deck, "--store", store.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("not started"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn reset_all_clears_a_seeded_store() {
    let dir = TempDir::new().unwrap();
    let store = write(
        dir.path(),
        "progress.json",
        r#"{"version":1,"cards":{"123":{"stage":2,"stage_entered_ms":0}}}"#,
    );
    let out = alix(&["reset", "--all", "--yes", "--store", &store]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("Reset 1 card(s)."),
        "stdout: {}",
        stdout(&out)
    );
    // The card is gone from the rewritten file.
    let after = std::fs::read_to_string(&store).unwrap();
    assert!(!after.contains("123"), "store still has the card: {after}");
}

#[test]
fn rejects_a_progress_file_from_a_newer_alix_without_touching_it() {
    // End-to-end version of the store unit test: an older alix run against a
    // store written by a newer alix must refuse — and never rewrite it.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.txt", VALID_DECK);
    let newer = r#"{"version":999,"cards":{}}"#;
    let store = write(dir.path(), "progress.json", newer);

    let out = alix(&["stats", &deck, "--store", &store]);
    assert!(!out.status.success(), "a newer store should be refused");
    assert!(
        stderr(&out).contains("upgrade alix"),
        "stderr: {}",
        stderr(&out)
    );
    // The file is byte-for-byte what it was — no silent downgrade.
    assert_eq!(newer, std::fs::read_to_string(&store).unwrap());
}

#[test]
fn a_corrupt_progress_file_fails_without_overwriting_it() {
    // A damaged store must not be silently replaced with an empty one — the
    // command fails and the bytes on disk are preserved for recovery.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.txt", VALID_DECK);
    let garbage = "{ this is not valid json";
    let store = write(dir.path(), "progress.json", garbage);

    let out = alix(&["stats", &deck, "--store", &store]);
    assert!(
        !out.status.success(),
        "a corrupt store should fail the command"
    );
    assert_eq!(garbage, std::fs::read_to_string(&store).unwrap());
}
