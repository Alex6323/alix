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
fn review_rejects_multiple_decks() {
    // One deck per session — merging several loose decks was removed. The guard
    // fires before any terminal is opened, so this is testable headless.
    let dir = TempDir::new().unwrap();
    let a = write(dir.path(), "a.txt", VALID_DECK);
    let b = write(dir.path(), "b.txt", VALID_DECK);
    let store = dir.path().join("p.json");
    let cfg = write(dir.path(), "cfg.toml", ""); // isolate from any global config
    let out = alix(&[
        "review",
        &a,
        &b,
        "--store",
        store.to_str().unwrap(),
        "--config",
        &cfg,
    ]);
    assert!(!out.status.success(), "reviewing two decks should error");
    assert!(
        stderr(&out).contains("one deck"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn review_rejects_a_workspace_directory() {
    // A workspace is reviewed member-by-member, never as a merged set.
    let dir = TempDir::new().unwrap();
    let ws = dir.path().join("eng");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("m.txt"), VALID_DECK).unwrap();
    let store = dir.path().join("p.json");
    let cfg = write(dir.path(), "cfg.toml", "");
    let out = alix(&[
        "review",
        ws.to_str().unwrap(),
        "--store",
        store.to_str().unwrap(),
        "--config",
        &cfg,
    ]);
    assert!(
        !out.status.success(),
        "reviewing a workspace dir should error"
    );
    assert!(
        stderr(&out).contains("workspace"),
        "stderr: {}",
        stderr(&out)
    );
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

/// A small trace deck over `src` (a single source file in the same dir).
fn trace_deck(dir: &Path) -> String {
    write(dir, "src.rs", "let a = b;\nuse_it(a);\n");
    write(
        dir,
        "t.txt",
        "% trace: how a moves into use_it\n% source: src.rs\n\
         # what happens to a?\n\tit is moved into use_it\n\t% at: 1-2\n",
    )
}

#[test]
fn exam_on_a_trace_no_longer_refuses_it() {
    // A trace used to be refused by `alix exam` ("walk it with `alix trace`").
    // Now its exam IS the compression, so the command enters the exam flow and
    // (headless, with no TTY) stops only at the terminal requirement — never the
    // old refusal.
    let dir = TempDir::new().unwrap();
    let deck = trace_deck(dir.path());
    let store = dir.path().join("p.json");
    let out = alix(&["exam", &deck, "--store", store.to_str().unwrap()]);
    let err = stderr(&out);
    assert!(!out.status.success(), "no TTY → it still can't run the UI");
    assert!(err.contains("needs a terminal"), "stderr: {err}");
    assert!(
        !err.contains("walk it") && !err.contains("is a trace"),
        "the trace is no longer refused: {err}"
    );
}

#[test]
fn exam_on_a_trace_is_gated_by_unfinished_prerequisites() {
    // A trace's exam runs in dependency order like a fact deck's: a sourced
    // prerequisite that hasn't been mastered locks it.
    let dir = TempDir::new().unwrap();
    write(dir.path(), "base-src.md", "the basics");
    write(dir.path(), "base.txt", "% source: base-src.md\n# b?\n\tb\n");
    write(dir.path(), "src.rs", "let a = b;\nuse_it(a);\n");
    let deck = write(
        dir.path(),
        "t.txt",
        "% trace: how a moves\n% source: src.rs\n% requires: base\n\
         # what happens?\n\tit moves\n\t% at: 1-2\n",
    );
    let store = dir.path().join("p.json");
    let out = alix(&["exam", &deck, "--store", store.to_str().unwrap()]);
    let err = stderr(&out);
    assert!(!out.status.success(), "a locked trace exam can't be sat");
    assert!(
        err.contains("prerequisites aren't finished"),
        "stderr: {err}"
    );
}
