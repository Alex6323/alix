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

/// Runs `alix <args...>` and returns its captured output. The child's `HOME` and
/// XDG dirs are pointed at a throwaway temp dir, so the suite never reads the
/// developer's real `~/.config/alix` or platform data dir — it's hermetic.
fn alix(args: &[&str]) -> Output {
    let home = TempDir::new().unwrap();
    Command::new(env!("CARGO_BIN_EXE_alix"))
        .args(args)
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path())
        .env("XDG_DATA_HOME", home.path())
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
    let out = alix(&["review", &a, &b, "--store", store.to_str().unwrap()]);
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
    let out = alix(&[
        "review",
        ws.to_str().unwrap(),
        "--store",
        store.to_str().unwrap(),
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

/// Writes an executable fake `claude` at `dir/fake-claude` that drains stdin
/// (so the prompt write never races into a broken pipe) then prints `reply`
/// verbatim, and returns its path. Mirrors `testutil::fake_reply`, but the CLI
/// suite drives the built binary as a subprocess so it can't reach that crate
/// helper — the fake is wired in via a `--config` TOML pointing `[ask] command`
/// at this script.
fn fake_claude(dir: &Path, reply: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let reply_path = dir.join("fake-reply.txt");
    std::fs::write(&reply_path, reply).unwrap();
    let script = dir.join("fake-claude");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncat >/dev/null; cat {}\n",
            reply_path.to_str().unwrap()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script.to_str().unwrap().to_string()
}

#[test]
fn augment_target_format_caches_a_reshape() {
    // `deck augment --target format` reshapes a badly-shaped plain card and
    // writes the result to the sidecar `augment.json` beside the store, without
    // touching the deck file. The Claude call is faked by a config-wired CLI.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "parts.txt", "# List the parts\n    A, B, C\n");
    // The model returns a structured reshape for card index 0: a list body and a
    // line-by-line mode suggestion.
    let cli = fake_claude(
        dir.path(),
        r#"{"0": {"back": ["A", "B", "C"], "mode": "line"}}"#,
    );
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let store = dir.path().join("p.json");
    let out = alix(&[
        "deck",
        "augment",
        &deck,
        "--target",
        "format",
        "--store",
        store.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    // The reshape is cached beside the store, not written back into the deck.
    let cached = std::fs::read_to_string(dir.path().join("augment.json")).unwrap();
    assert!(cached.contains("\"A\""), "augment.json: {cached}");
    assert!(cached.contains("LineByLine"), "augment.json: {cached}");
    let deck_after = std::fs::read_to_string(&deck).unwrap();
    assert_eq!(
        deck_after, "# List the parts\n    A, B, C\n",
        "deck untouched"
    );
}

#[test]
fn url_source_exam_on_codex_refuses_cleanly() {
    // The Codex backend runs read-only with no network, so it can't fetch a URL
    // `% source:`. `alix exam` must refuse before touching anything — a plain
    // message naming the gap and the fix, and it writes no progress store.
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "web.txt",
        "% source: https://example.org/page\n# q?\n\ta\n",
    );
    let config = write(dir.path(), "config.toml", "[ask]\nbackend = \"codex\"\n");
    let store = dir.path().join("p.json");
    let out = alix(&[
        "exam",
        &deck,
        "--store",
        store.to_str().unwrap(),
        "--config",
        &config,
    ]);
    let err = stderr(&out);
    assert!(!out.status.success(), "the exam must refuse, stderr: {err}");
    assert!(
        err.contains("codex") && err.contains("can't fetch"),
        "refusal must name the gap: {err}"
    );
    // Refused before any side effect: no progress store was written.
    assert!(!store.exists(), "a refused exam must not write the store");
}

#[test]
fn missing_backend_reports_install_hint() {
    // Pointing `[ask] command` at a nonexistent binary yields the install hint,
    // not a raw OS error. Uses `deck generate` (no TTY needed) to reach the runner.
    let dir = TempDir::new().unwrap();
    let config = write(
        dir.path(),
        "config.toml",
        "[ask]\ncommand = \"/nonexistent/claude-xyz\"\ntimeout_secs = 5\n",
    );
    let out = alix(&[
        "deck",
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--print",
    ]);
    let err = stderr(&out);
    assert!(!out.status.success(), "a missing backend must fail: {err}");
    assert!(
        err.contains("is it installed"),
        "should hint at installation: {err}"
    );
}

/// Creates `n` small files in `dir`, each `bytes` bytes, to simulate a large
/// source tree.
fn make_large_tree(dir: &std::path::Path, n: usize, bytes: usize) {
    let content = vec![0u8; bytes];
    for i in 0..n {
        std::fs::write(dir.join(format!("f{i}.bin")), &content).unwrap();
    }
}

#[test]
fn oversized_local_source_without_yes_bails_with_guidance() {
    // An oversized source tree with no TTY and no --yes must bail.
    let dir = TempDir::new().unwrap();
    // Write enough bytes to exceed the 5 MB default threshold.
    make_large_tree(dir.path(), 10, 600_000); // 6 MB total
    let config = write(
        dir.path(),
        "config.toml",
        // A nonexistent backend: we never reach the model, but we need the guard to fire.
        "[ask]\ncommand = \"/nonexistent/claude-xyz\"\ntimeout_secs = 5\n",
    );
    let src = dir.path().to_str().unwrap();
    let out = alix(&[
        "deck",
        "generate",
        src,
        "--config",
        &config,
        "--print",
    ]);
    let err = stderr(&out);
    assert!(!out.status.success(), "should fail without --yes: {err}");
    // The error must name the guard condition and point at the fix.
    assert!(
        err.contains("--yes"),
        "error must mention --yes: {err}"
    );
    assert!(
        err.contains("large source tree") || err.contains("files"),
        "error must describe the source size: {err}"
    );
}

#[test]
fn oversized_local_source_with_yes_proceeds_past_the_guard() {
    // With --yes the guard is bypassed and we reach the backend (which may fail
    // because the binary doesn't exist — that's fine; what matters is we got
    // past the size check).
    let dir = TempDir::new().unwrap();
    make_large_tree(dir.path(), 10, 600_000); // 6 MB total
    let config = write(
        dir.path(),
        "config.toml",
        "[ask]\ncommand = \"/nonexistent/claude-xyz\"\ntimeout_secs = 5\n",
    );
    let src = dir.path().to_str().unwrap();
    let out = alix(&[
        "deck",
        "generate",
        src,
        "--yes",
        "--config",
        &config,
        "--print",
    ]);
    let err = stderr(&out);
    // The error must NOT be the guard refusal — it should be the missing-binary
    // hint (or something from the model runner).
    assert!(
        !err.contains("pass --yes to proceed"),
        "guard must not fire with --yes: {err}"
    );
    // It should have reached the backend and failed there instead.
    assert!(
        err.contains("is it installed") || err.contains("nonexistent"),
        "should reach the backend: {err}"
    );
}

#[test]
fn undersized_local_source_proceeds_without_yes() {
    // A source tree under the threshold passes the guard silently without --yes.
    let dir = TempDir::new().unwrap();
    write(dir.path(), "small.txt", "hello world\n");
    let config = write(
        dir.path(),
        "config.toml",
        "[ask]\ncommand = \"/nonexistent/claude-xyz\"\ntimeout_secs = 5\n",
    );
    let out = alix(&[
        "deck",
        "generate",
        dir.path().to_str().unwrap(),
        "--config",
        &config,
        "--print",
    ]);
    let err = stderr(&out);
    // Must not hit the guard — passes through to the backend.
    assert!(
        !err.contains("pass --yes to proceed"),
        "guard must not fire for small trees: {err}"
    );
}
