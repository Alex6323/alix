//! End-to-end CLI integration tests: each runs the built `alix` binary as a
//! subprocess against temp decks and a temp state root, asserting on exit
//! status and output. Unlike `tests/calibrate.rs` these are fully deterministic
//! (no real Claude) so they run in CI on every `make check`.
//!
//! A recurring property here is that a damaged progress document fails *safely*:
//! the command errors and the document on disk is left exactly as it was, never
//! silently overwritten with an empty store.
//!
//! Unix-only: the fake AI backend is a `/bin/sh` script. The Windows CI job
//! runs the lib persistence suites instead.
#![cfg(unix)]

use std::{
    path::{Path, PathBuf},
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

/// Like [`alix`], but with a caller-supplied (long-lived) home directory
/// instead of an ephemeral one — for a test that needs to inspect what landed
/// in the default decks/config dir, or re-invoke against the same state — plus
/// arbitrary extra env vars overlaid last. Used to make an external-binary
/// dependency (e.g. `wormhole`) deterministically absent via a stripped `PATH`,
/// without touching this test process's own environment.
fn alix_env(args: &[&str], home: &Path, extra_env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_alix"));
    cmd.args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_DATA_HOME", home);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.output().expect("failed to run the alix binary")
}

fn test_config_dir(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support/alix")
    }
    #[cfg(not(target_os = "macos"))]
    {
        home.join("alix")
    }
}

/// Writes `contents` to `dir/name` and returns its path as a string.
fn write(dir: &Path, name: &str, contents: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path.to_str().unwrap().to_string()
}

fn deck_store(deck: &str, state_root: &Path) -> alix::store::Store {
    alix::state::open_store(Path::new(deck), state_root).unwrap()
}

fn decks_store(decks: &[&str], state_root: &Path) -> alix::store::Store {
    let paths = decks.iter().map(PathBuf::from).collect::<Vec<_>>();
    alix::state::open_stores(&paths, state_root).unwrap()
}

fn augmentation_text(deck: &str) -> String {
    let deck = alix::deck::Deck::load(deck).unwrap();
    let deck_id = deck.deck_token.as_deref().unwrap();
    let path = alix::workspace::WorkspaceFiles::for_deck(&deck.path).augment_for(deck_id);
    std::fs::read_to_string(path).unwrap()
}

fn write_progress_document(
    state_root: &Path,
    deck_id: &str,
    subject: &str,
    cards: &str,
) -> PathBuf {
    let path = alix::state::UserFiles::new(state_root).progress_for(deck_id);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        format!(
            "{{\"version\":1,\"deck_id\":\"{deck_id}\",\"subject\":\"{subject}\",\
             \"revision\":1,\"cards\":{{{cards}}},\"records\":{{}},\
             \"virtual_cards\":{{}},\"writer\":null}}"
        ),
    )
    .unwrap();
    path
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

const VALID_DECK: &str = "---\nformat-version: 1\nid: \"deck-mathdeck\"\n---\n## What is 2 + 2? <!-- id: card-math1 -->\n4\n";

#[test]
fn profile_add_list_and_remove_are_hermetic() {
    let home = TempDir::new().unwrap();
    let decks = home.path().join("decks-x");
    let decks = decks.to_str().unwrap();

    let added = alix_env(
        &["profile", "add", "x", "--decks", decks, "--port", "7002"],
        home.path(),
        &[],
    );
    assert!(added.status.success(), "stderr: {}", stderr(&added));
    // `directories` resolves the config dir per platform: XDG_CONFIG_HOME
    // (redirected to the temp home) on Linux, Library/... under HOME on macOS.
    // Hermeticity holds on both; only the asserted path differs.
    #[cfg(target_os = "macos")]
    let profile = home
        .path()
        .join("Library/Application Support/alix/profiles/x.toml");
    #[cfg(not(target_os = "macos"))]
    let profile = home.path().join("alix/profiles/x.toml");
    assert!(profile.exists());

    let listed = alix_env(&["profile", "list"], home.path(), &[]);
    assert!(listed.status.success(), "stderr: {}", stderr(&listed));
    assert!(stdout(&listed).contains("x"), "stdout: {}", stdout(&listed));
    assert!(
        stdout(&listed).contains("7002"),
        "stdout: {}",
        stdout(&listed)
    );

    let removed = alix_env(&["profile", "remove", "x", "--yes"], home.path(), &[]);
    assert!(removed.status.success(), "stderr: {}", stderr(&removed));

    let listed = alix_env(&["profile", "list"], home.path(), &[]);
    assert!(listed.status.success(), "stderr: {}", stderr(&listed));
    assert!(
        stdout(&listed).contains("no profiles yet"),
        "stdout: {}",
        stdout(&listed)
    );
}

#[test]
fn profile_default_cli_sets_shows_and_clears_the_marker() {
    let home = TempDir::new().unwrap();
    let decks = home.path().join("decks-x");
    let decks = decks.to_str().unwrap();
    let added = alix_env(&["profile", "add", "x", "--decks", decks], home.path(), &[]);
    assert!(added.status.success(), "stderr: {}", stderr(&added));

    let show_empty = alix_env(&["profile", "default"], home.path(), &[]);
    assert!(
        show_empty.status.success(),
        "stderr: {}",
        stderr(&show_empty)
    );
    assert_eq!("none\n", stdout(&show_empty));

    let set = alix_env(&["profile", "default", "x"], home.path(), &[]);
    assert!(set.status.success(), "stderr: {}", stderr(&set));
    assert_eq!("default profile: x\n", stdout(&set));

    let show = alix_env(&["profile", "default"], home.path(), &[]);
    assert!(show.status.success(), "stderr: {}", stderr(&show));
    assert_eq!("x\n", stdout(&show));

    let clear = alix_env(&["profile", "default", "--clear"], home.path(), &[]);
    assert!(clear.status.success(), "stderr: {}", stderr(&clear));
    assert_eq!("default profile cleared\n", stdout(&clear));

    let show_cleared = alix_env(&["profile", "default"], home.path(), &[]);
    assert!(
        show_cleared.status.success(),
        "stderr: {}",
        stderr(&show_cleared)
    );
    assert_eq!("none\n", stdout(&show_cleared));
}

#[test]
fn profile_launch_reports_the_exact_missing_named_profile() {
    let home = TempDir::new().unwrap();

    let out = alix_env(&["profile", "missing"], home.path(), &[]);

    assert!(!out.status.success(), "stdout: {}", stdout(&out));
    assert!(
        stderr(&out).contains("no profile `missing`"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn launch_all_without_profiles_reports_the_creation_command() {
    let home = TempDir::new().unwrap();

    let out = alix_env(&["--launch-all"], home.path(), &[]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        "no profiles to launch; create one with `alix profile add <name>`\n",
        stdout(&out)
    );
}

#[test]
fn bare_launch_uses_the_named_default_profile_config() {
    let home = TempDir::new().unwrap();
    let config_dir = test_config_dir(home.path());
    let profiles = config_dir.join("profiles");
    std::fs::create_dir_all(&profiles).unwrap();
    std::fs::write(
        profiles.join("named.toml"),
        "[review]\nnamed_profile_only = true\n",
    )
    .unwrap();
    std::fs::write(profiles.join("default"), "named\n").unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        "[review]\nglobal_config_only = true\n",
    )
    .unwrap();

    let out = alix_env(&[], home.path(), &[]);

    assert!(!out.status.success(), "stdout: {}", stdout(&out));
    let error = stderr(&out);
    assert!(error.contains("named.toml"), "stderr: {error}");
    assert!(error.contains("named_profile_only"), "stderr: {error}");
    assert!(!error.contains("global_config_only"), "stderr: {error}");
}

#[test]
fn an_explicit_config_bypasses_the_default_profile() {
    let home = TempDir::new().unwrap();
    let config_dir = test_config_dir(home.path());
    let profiles = config_dir.join("profiles");
    std::fs::create_dir_all(&profiles).unwrap();
    std::fs::write(
        profiles.join("named.toml"),
        "[review]\ndefault_profile_only = true\n",
    )
    .unwrap();
    std::fs::write(profiles.join("default"), "named\n").unwrap();
    let explicit = write(
        home.path(),
        "explicit.toml",
        "[review]\nexplicit_config_only = true\n",
    );

    let out = alix_env(&["--config", &explicit], home.path(), &[]);

    assert!(!out.status.success(), "stdout: {}", stdout(&out));
    let error = stderr(&out);
    assert!(error.contains("explicit.toml"), "stderr: {error}");
    assert!(error.contains("explicit_config_only"), "stderr: {error}");
    assert!(!error.contains("default_profile_only"), "stderr: {error}");
}

#[test]
fn check_accepts_a_valid_deck() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let out = alix(&["doctor", &deck]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("1 cards"), "stdout: {}", stdout(&out));
}

#[test]
fn deck_copy_lands_the_public_deck_and_reports_the_boundary() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("source");
    let destination = dir.path().join("destination");
    std::fs::create_dir_all(source.join("decks")).unwrap();
    std::fs::create_dir_all(destination.join("decks")).unwrap();
    std::fs::write(source.join("alix.toml"), "").unwrap();
    std::fs::write(destination.join("alix.toml"), "").unwrap();
    let deck = write(
        &source.join("decks"),
        "facts.md",
        "---\nformat-version: 1\nid: deck-deck1\n---\n## q\nanswer\n<!-- id: card-card1 -->\n",
    );

    let out = alix(&["deck", "copy", &deck, destination.to_str().unwrap()]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("progress moved: no"),
        "stdout: {}",
        stdout(&out)
    );
    assert!(destination.join("decks/facts.md").is_file());
    assert!(Path::new(&deck).is_file());
}

#[test]
fn deck_move_requires_confirmation_and_reports_progress() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("source");
    let destination = dir.path().join("destination");
    std::fs::create_dir_all(source.join("decks")).unwrap();
    std::fs::create_dir_all(destination.join("decks")).unwrap();
    std::fs::write(source.join("alix.toml"), "").unwrap();
    std::fs::write(destination.join("alix.toml"), "").unwrap();
    let deck = write(
        &source.join("decks"),
        "facts.md",
        "---\nformat-version: 1\nid: deck-deck1\n---\n## q\nanswer\n<!-- id: card-card1 -->\n",
    );
    write_progress_document(&source, "deck-deck1", "facts.md", "");

    let refused = alix(&["deck", "move", &deck, destination.to_str().unwrap()]);

    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("pass --yes"),
        "stderr: {}",
        stderr(&refused)
    );
    assert!(Path::new(&deck).is_file());

    let moved = alix(&[
        "deck",
        "move",
        &deck,
        destination.to_str().unwrap(),
        "--yes",
    ]);

    assert!(moved.status.success(), "stderr: {}", stderr(&moved));
    assert!(
        stdout(&moved).contains("progress moved: yes"),
        "stdout: {}",
        stdout(&moved)
    );
    assert!(!Path::new(&deck).exists());
    assert!(destination.join("decks/facts.md").is_file());
    assert!(destination.join("progress/deck-deck1.json").is_file());
}

#[test]
fn a_deck_file_argument_errors_with_a_picker_pointer() {
    // `alix <deck>` was removed — the picker is the one way into a review. The
    // guard fires before any server binds, so this is testable headless.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "a.md", VALID_DECK);
    let out = alix(&[&deck]);
    assert!(!out.status.success(), "a deck-file argument should error");
    assert!(
        stderr(&out).contains("was removed"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn the_review_subcommand_is_gone() {
    // `alix review x.md` now parses as a launcher path plus an unexpected
    // extra positional — the subcommand no longer exists.
    let out = alix(&["review", "x.md"]);
    assert!(
        !out.status.success(),
        "the review subcommand should be gone"
    );
}

#[test]
fn check_rejects_a_malformed_deck() {
    let dir = TempDir::new().unwrap();
    // A card front with no answer line is a parse error.
    let deck = write(dir.path(), "broken.md", "## a front with no answer\n");
    let out = alix(&["doctor", &deck]);
    assert!(
        !out.status.success(),
        "a malformed deck should fail the check"
    );
    assert!(stderr(&out).contains("error:"), "stderr: {}", stderr(&out));
}

#[test]
fn doctor_warns_about_a_malformed_deadline_without_failing() {
    let dir = TempDir::new().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("alix.toml"), "").unwrap();
    std::fs::write(
        ws.join("alix.local.toml"),
        "[review]\ndeadline = \"soonish\"\n",
    )
    .unwrap();
    std::fs::create_dir(ws.join("decks")).unwrap();
    write(&ws.join("decks"), "cards.md", VALID_DECK);
    let out = alix(&["doctor", ws.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "warnings should not fail the doctor check"
    );
    let err = stderr(&out);
    assert!(err.contains("deadline"), "stderr: {err}");
    assert!(err.contains("warning"), "stderr: {err}");
}

#[test]
fn doctor_nudges_a_long_source_list_toward_its_common_root() {
    let dir = TempDir::new().unwrap();
    for name in ["a.rs", "b.rs", "c.rs", "d.rs"] {
        std::fs::write(dir.path().join(name), "x\n").unwrap();
    }
    let deck = write(
        dir.path(),
        "cards.md",
        "---\nformat-version: 1\nid: \"deck-deck1\"\nsource:\n  - a.rs\n  - b.rs\n  - c.rs\n  - d.rs\n---\n## q\na\n<!-- id: card-card1 -->\n",
    );
    let out = alix(&["doctor", &deck]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("4 expressions"), "stderr: {err}");
    assert!(err.contains("common root"), "stderr: {err}");
}

#[test]
fn workspace_init_writes_both_documented_manifests() {
    let dir = TempDir::new().unwrap();
    let ws = dir.path().join("fresh");
    let out = alix(&["workspace", "init", ws.to_str().unwrap(), "--title", "T"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let manifest = std::fs::read_to_string(ws.join("alix.toml")).unwrap();
    assert!(manifest.contains("title = \"T\""), "{manifest}");
    assert!(manifest.contains("[defaults]"), "headers stay uncommented");
    let local = std::fs::read_to_string(ws.join("alix.local.toml")).unwrap();
    assert!(local.contains("[review]"), "headers stay uncommented");
    assert!(local.contains("never shared"), "{local}");
    assert!(ws.join("decks").is_dir());
    assert!(ws.join("assets").is_dir());
}

#[test]
fn workspace_deadline_shows_sets_and_clears() {
    let dir = TempDir::new().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"Ws\"\n").unwrap();
    std::fs::write(
        ws.join("decks/cards.md"),
        "---\nformat-version: 1\nid: \"deck-cards\"\n---\n## Q?\nA\n",
    )
    .unwrap();

    let out = alix(&["workspace", "deadline", ws.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("no deadline"),
        "stdout: {}",
        stdout(&out)
    );

    let out = alix(&["workspace", "deadline", ws.to_str().unwrap(), "2099-01-02"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        std::fs::read_to_string(ws.join("alix.local.toml"))
            .unwrap()
            .contains("2099-01-02")
    );

    let out = alix(&["workspace", "deadline", ws.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let show_output = stdout(&out);
    assert!(show_output.contains("2099-01-02"), "stdout: {show_output}");
    assert!(show_output.contains("days"), "stdout: {show_output}");

    let out = alix(&["workspace", "deadline", ws.to_str().unwrap(), "clear"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        !std::fs::read_to_string(ws.join("alix.local.toml"))
            .unwrap()
            .contains("deadline")
    );

    let out = alix(&["workspace", "deadline", ws.to_str().unwrap(), "not-a-date"]);
    assert!(!out.status.success(), "stderr: {}", stderr(&out));
}

#[test]
fn workspace_deadline_labels_today_and_past_day_boundaries_exactly() {
    let dir = TempDir::new().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"Ws\"\n").unwrap();
    let today = alix::time::local_date(alix::time::now_ms());
    let yesterday = today.pred_opt().unwrap();

    for (date, expected) in [(yesterday, "(was due 1 day ago)"), (today, "(0 days left)")] {
        let date = date.format("%Y-%m-%d").to_string();
        let set = alix(&["workspace", "deadline", ws.to_str().unwrap(), &date]);
        assert!(set.status.success(), "{date}: {}", stderr(&set));

        let show = alix(&["workspace", "deadline", ws.to_str().unwrap()]);
        assert!(show.status.success(), "{date}: {}", stderr(&show));
        assert!(
            stdout(&show).contains(expected),
            "{date}: expected {expected:?}, stdout: {}",
            stdout(&show)
        );
    }
}

#[test]
fn workspace_deadline_rejects_non_workspace_non_decks_dir() {
    let dir = TempDir::new().unwrap();
    let empty_dir = dir.path().join("empty");
    std::fs::create_dir(&empty_dir).unwrap();

    let out = alix(&["workspace", "deadline", empty_dir.to_str().unwrap()]);
    assert!(!out.status.success(), "stderr: {}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("not a workspace")
            || err.contains("not a decks folder")
            || err.contains("not a workspace or decks folder"),
        "stderr should mention neither workspace nor decks folder: {err}"
    );
}

#[test]
fn workspace_deadline_rejects_a_decks_folder_without_a_manifest() {
    // DECISION 2026-07-15: deadline keys apply only inside a real workspace
    // (manifest present); a plain decks folder is rejected and pointed at the
    // upgrade path, rather than silently accepted like before.
    let dir = TempDir::new().unwrap();
    let plain = dir.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    write(&plain, "cards.md", "## Q?\nA\n");

    let out = alix(&["workspace", "deadline", plain.to_str().unwrap()]);
    assert!(!out.status.success(), "stderr: {}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("workspace init"), "stderr: {err}");
}

#[test]
fn stats_on_a_folder_reports_every_deck_inside() {
    let dir = TempDir::new().unwrap();
    write(
        dir.path(),
        "alpha.md",
        "---\nformat-version: 1\nid: deck-alpha\n---\n## a? <!-- id: card-a1 -->\na\n",
    );
    write(
        dir.path(),
        "beta.md",
        "---\nformat-version: 1\nid: deck-beta\n---\n## b? <!-- id: card-b1 -->\nb\n",
    );
    let store = dir.path().join("state");
    let out = alix(&[
        "stats",
        dir.path().to_str().unwrap(),
        "--store",
        store.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("alpha"), "stdout: {text}");
    assert!(text.contains("beta"), "stdout: {text}");
}

#[test]
fn reset_on_a_workspace_clears_every_member_in_its_own_store() {
    // A workspace target expands to its member decks and hits the workspace's
    // own per-deck progress documents, including the mastered flag.
    let dir = TempDir::new().unwrap();
    let ws = dir.path().join("eng");
    let members = ws.join("decks");
    std::fs::create_dir_all(&members).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"Eng\"\n").unwrap();
    let a = write(
        &members,
        "a.md",
        "---\nformat-version: 1\nid: deck-decka\n---\n## qa <!-- id: card-qa1 -->\nans-a\n",
    );
    let b = write(
        &members,
        "b.md",
        "---\nformat-version: 1\nid: deck-deckb\n---\n## qb <!-- id: card-qb1 -->\nans-b\n",
    );
    let store_path = ws.clone();
    let mut store = decks_store(&[&a, &b], &store_path);
    for deck in [&a, &b] {
        let cards = alix::parser::parse_str(
            Path::new(deck).file_name().unwrap().to_str().unwrap(),
            &std::fs::read_to_string(deck).unwrap(),
        )
        .unwrap();
        store.get_or_insert(&cards[0].id().unwrap(), 0);
    }
    store.set_deck_mastered("deck-decka", 0);
    store.save().unwrap();

    let out = alix(&["reset", ws.to_str().unwrap(), "--yes"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let reloaded = decks_store(&[&a, &b], &store_path);
    assert_eq!(0, reloaded.len(), "member card progress should be gone");
    assert!(
        !reloaded.deck_mastered("deck-decka"),
        "the mastered flag should be cleared"
    );
}

#[test]
fn stats_reports_a_fresh_deck_against_an_empty_store() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let store = dir.path().join("state");
    let out = alix(&["stats", &deck, "--store", store.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("not started"),
        "stdout: {}",
        stdout(&out)
    );
    assert!(
        !stdout(&out).contains("reviews:"),
        "an empty review total must not print a percentage: {}",
        stdout(&out)
    );
}

#[test]
fn stats_aggregates_authored_and_virtual_due_windows_and_review_totals() {
    let dir = TempDir::new().unwrap();
    let deck_text = "---\nformat-version: 1\nid: deck-statsall\n---\n\
## Q1 <!-- id: card-stats1 -->\nA1\n\n\
## Q2 <!-- id: card-stats2 -->\nA2\n\n\
## Q3 <!-- id: card-stats3 -->\nA3\n\n\
## Q4 <!-- id: card-stats4 -->\nA4\n";
    let deck = write(dir.path(), "all-stats.md", deck_text);
    let parsed = alix::deck::Deck::load(&deck).unwrap();
    let state_root = dir.path().join("state");
    let mut store = deck_store(&deck, &state_root);
    let now = alix::time::now_ms();
    let due_times = [
        now.saturating_sub(1_000),
        now + 6 * 60 * 60 * 1_000,
        now + 12 * 60 * 60 * 1_000,
        now + 48 * 60 * 60 * 1_000,
    ];
    let review_totals = [(2, 1), (3, 2), (1, 0), (4, 4)];
    for ((card, due_ms), (reviews, passes)) in parsed.cards.iter().zip(due_times).zip(review_totals)
    {
        let state = store.get_or_insert(&card.id().unwrap(), now);
        state.recall = Some(alix::store::FsrsState {
            state: 2,
            due_ms,
            scheduled_days: 1,
            ..Default::default()
        });
        state.total_reviews = reviews;
        state.total_passes = passes;
    }

    for (id, due_ms) in [
        ("card-virtual-now", now.saturating_sub(1_000)),
        ("card-virtual-soon", now + 6 * 60 * 60 * 1_000),
    ] {
        store.insert_virtual(alix::store::VirtualCard {
            id: id.to_string(),
            kind: alix::store::VirtualKind::Remediation,
            deck: "deck-statsall".to_string(),
            text: format!("## virtual <!-- id: {id} -->\nanswer\n"),
            created_ms: now,
        });
        store.get_or_insert(id, now).recall = Some(alix::store::FsrsState {
            state: 2,
            due_ms,
            scheduled_days: 1,
            ..Default::default()
        });
    }
    store.save().unwrap();

    let out = alix(&["stats", &deck, "--store", state_root.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let result = stdout(&out);
    for exact in [
        "  state:   finished ✓",
        "  due:     2 now, 3 within 24h",
        "  due now (recall):      2",
        "  reviews: 10 total, 70% passed",
    ] {
        assert!(result.contains(exact), "missing {exact:?}: {result}");
    }
}

#[test]
fn stats_reserves_mastered_for_a_recorded_mastery_marker() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let state_root = dir.path().join("state");
    let mut store = deck_store(&deck, &state_root);
    store.set_deck_mastered("deck-mathdeck", alix::time::now_ms());
    store.save().unwrap();

    let out = alix(&["stats", &deck, "--store", state_root.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("  state:   mastered ✓"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn reset_all_clears_a_seeded_store() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let state_root = dir.path().join("state");
    let mut store = deck_store(&deck, &state_root);
    store.get_or_insert("card-math1", 0);
    store.save().unwrap();
    let out = alix(&[
        "reset",
        "--all",
        "--yes",
        "--store",
        state_root.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("Reset 1 card(s)."),
        "stdout: {}",
        stdout(&out)
    );
    let reloaded = deck_store(&deck, &state_root);
    assert!(reloaded.get("card-math1").is_none());
}

/// A minimal virtual card owned by `deck_id`, for seeding a store directly
/// via the lib rather than hand-authoring its on-disk JSON shape. Its id is the
/// `Card::id` of the card `parse(deck_id, text)` yields, identical to a deck
/// card. The literal `<!-- id: -->` token is derived from `deck_id`: identity is
/// the token now, so two decks' sample cards must not share one.
fn sample_virtual_card(deck_id: &str) -> alix::store::VirtualCard {
    let token: String = format!(
        "v{}",
        deck_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase()
    );
    let text = format!("## front <!-- id: card-{token} -->\nback\n");
    let id = alix::parser::parse_str(deck_id, &text).unwrap()[0]
        .id()
        .unwrap();
    alix::store::VirtualCard {
        id,
        kind: alix::store::VirtualKind::Remediation,
        deck: deck_id.to_string(),
        text,
        created_ms: 0,
    }
}

#[test]
fn reset_all_clears_virtual_cards() {
    // A store holding ONLY virtual cards must still be reset by `--all`: the
    // count sees the virtual card's schedule in `store.cards` (seeded beside the
    // sidecar entry), and the clear must also drop its sidecar content.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let store_path = dir.path().join("state");
    let mut store = deck_store(&deck, &store_path);
    let vc = sample_virtual_card("math.md");
    let id = vc.id.clone();
    store.insert_virtual(vc);
    store.get_or_insert(&id, 0); // the virtual card's schedule lives in store.cards
    store.save().unwrap();

    let out = alix(&[
        "reset",
        "--all",
        "--yes",
        "--store",
        store_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        !stdout(&out).contains("No stored progress"),
        "a virtual-only store wrongly reported nothing to reset: {}",
        stdout(&out)
    );

    let reloaded = deck_store(&deck, &store_path);
    assert_eq!(0, reloaded.iter_virtual_cards().count());
}

#[test]
fn orphans_are_never_auto_pruned_and_reset_orphans_clears_them() {
    // Orphaned progress: a store key matching no live card, or a whole
    // progress document matching no live deck (a stripped id comment, a
    // hand-deleted deck), is evidence. A normal reset never sweeps it; only
    // the explicit `reset --orphans` does. A deck-level orphan is now a
    // whole `progress/<id>.json` document (deck-level state is single-valued
    // per document, not a shared map), so a second real deck keeps
    // `reset --orphans` from collapsing to a single-document open that would
    // never see it.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK); // card id `math1`
    write(
        dir.path(),
        "other.md",
        "---\nformat-version: 1\nid: \"deck-otherdeck\"\n---\n## other <!-- id: card-other1 -->\nb\n",
    );
    let store_path = dir.path().join("state");

    let mut store = deck_store(&deck, &store_path);
    store.get_or_insert("card-math1", 0); // the live card
    store.get_or_insert("orphan1", 0); // an orphaned card key
    store.save().unwrap();
    // A hand-deleted deck's progress document: a deck_id with no matching
    // `.md` file, carrying non-empty deck-level state.
    let ghost = write_progress_document(&store_path, "deck-ghost", "ghost.md", "");
    let ghost_text = std::fs::read_to_string(&ghost).unwrap().replace(
        "\"cards\":{}",
        "\"cards\":{},\"deck\":{\"last_depth\":\"recall\"}",
    );
    std::fs::write(&ghost, ghost_text).unwrap();

    // A normal full-deck reset clears the live card but leaves the orphans,
    // proof they are never auto-pruned.
    let out = alix(&[
        "reset",
        &deck,
        "--yes",
        "--store",
        store_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let after = std::fs::read_to_string(store.path()).unwrap();
    assert!(
        !after.contains("card-math1"),
        "the live card should be reset: {after}"
    );
    assert!(
        after.contains("orphan1"),
        "the orphan card must survive: {after}"
    );
    assert!(
        std::fs::read_to_string(&ghost)
            .unwrap()
            .contains("last_depth"),
        "the orphaned deck's document must survive a normal reset"
    );

    // `reset --orphans` over the folder clears exactly the orphaned keys.
    let out = alix(&[
        "reset",
        "--orphans",
        dir.path().to_str().unwrap(),
        "--yes",
        "--store",
        store_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("Reset 2 orphaned key(s)."),
        "stdout: {}",
        stdout(&out)
    );
    let after = std::fs::read_to_string(store.path()).unwrap();
    assert!(
        !after.contains("orphan1"),
        "orphan card not cleared: {after}"
    );
    assert!(
        !std::fs::read_to_string(&ghost)
            .unwrap()
            .contains("last_depth"),
        "orphan deck document not cleared"
    );
}

#[test]
fn reset_orphans_clears_an_orphaned_document_whatever_the_live_deck_count() {
    // `doctor` scans the target root's whole aggregate, so `reset --orphans`
    // must too: reading only the named deck's own document hides the orphan
    // whenever the target holds exactly one live deck.
    for live_decks in [1usize, 2] {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("state");
        let mut live: Vec<(PathBuf, String)> = Vec::new();
        for index in 0..live_decks {
            let deck = write(
                dir.path(),
                &format!("live{index}.md"),
                &format!(
                    "---\nformat-version: 1\nid: \"deck-live{index}\"\n---\n\
                     ## question {index} <!-- id: card-live{index} -->\nanswer\n"
                ),
            );
            let mut store = deck_store(&deck, &store_path);
            let card = format!("card-live{index}");
            store.get_or_insert(&card, 0);
            store.save().unwrap();
            live.push((store.path().to_path_buf(), card));
        }
        let ghost_path = alix::state::UserFiles::new(&store_path).progress_for("deck-ghost");
        let mut ghost =
            alix::store::Store::open_deck(&ghost_path, "deck-ghost", "ghost.md").unwrap();
        ghost.get_or_insert("card-ghost1", 0);
        ghost.save().unwrap();

        let reset = &[
            "reset",
            "--orphans",
            dir.path().to_str().unwrap(),
            "--yes",
            "--store",
            store_path.to_str().unwrap(),
        ];
        let out = alix(reset);

        assert!(
            out.status.success(),
            "{live_decks} live deck(s): stderr: {}",
            stderr(&out)
        );
        assert!(
            stdout(&out).contains("Reset 1 orphaned key(s)."),
            "{live_decks} live deck(s): the deleted deck's key was not reported: {}",
            stdout(&out)
        );
        let ghost_text = std::fs::read_to_string(&ghost_path).unwrap();
        assert!(
            !ghost_text.contains("card-ghost1"),
            "{live_decks} live deck(s): the orphaned key survives in {}: {ghost_text}",
            ghost_path.display()
        );
        for (path, card) in &live {
            let text = std::fs::read_to_string(path).unwrap();
            assert!(
                text.contains(card.as_str()),
                "{live_decks} live deck(s): live progress for {card} was pruned: {text}"
            );
        }

        let out = alix(reset);
        assert!(
            stdout(&out).contains("No orphaned progress to reset."),
            "{live_decks} live deck(s): a swept target still reported orphans: {}",
            stdout(&out)
        );
    }
}

#[test]
fn reset_orphans_refuses_while_any_deck_like_file_in_the_target_cannot_be_read() {
    // A deck the parser rejects hides whatever ids it holds, so every key in
    // the target would look orphaned: the sweep stops instead of pruning.
    for (shape, broken) in [
        (
            "an invalid card id",
            "---\nformat-version: 1\nid: \"deck-b\"\n---\n## q <!-- id: nope -->\na\n",
        ),
        (
            "unclosed frontmatter",
            "---\nformat-version: 1\nid: \"deck-b\"\n## q <!-- id: card-b1 -->\na\n",
        ),
        ("a front without an answer", "## q <!-- id: card-b1 -->\n"),
    ] {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("state");
        let live = write(dir.path(), "live.md", VALID_DECK);
        let mut store = deck_store(&live, &store_path);
        store.get_or_insert("card-math1", 0);
        store.save().unwrap();
        let live_progress = store.path().to_path_buf();
        let ghost_path =
            write_progress_document(&store_path, "deck-ghost", "ghost.md", "\"card-ghost1\":{}");
        write(dir.path(), "broken.md", broken);
        let before = (
            std::fs::read_to_string(&live_progress).unwrap(),
            std::fs::read_to_string(&ghost_path).unwrap(),
        );

        let out = alix(&[
            "reset",
            "--orphans",
            dir.path().to_str().unwrap(),
            "--yes",
            "--store",
            store_path.to_str().unwrap(),
        ]);

        assert!(
            !out.status.success(),
            "{shape}: the sweep ran anyway: {}",
            stdout(&out)
        );
        assert_eq!(
            before.0,
            std::fs::read_to_string(&live_progress).unwrap(),
            "{shape}: live progress changed"
        );
        assert_eq!(
            before.1,
            std::fs::read_to_string(&ghost_path).unwrap(),
            "{shape}: the orphaned document was pruned on an unreadable target"
        );
    }
}

#[test]
fn reset_orphans_spares_the_cards_of_a_deck_that_lost_its_frontmatter_id() {
    // Stripping the `id:` line drops the file out of the initialized listing;
    // its cards are still live cards, not orphans.
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("state");
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let mut store = deck_store(&deck, &store_path);
    store.get_or_insert("card-math1", 0);
    store.save().unwrap();
    let progress = store.path().to_path_buf();
    write(
        dir.path(),
        "math.md",
        "## What is 2 + 2? <!-- id: card-math1 -->\n4\n",
    );

    let out = alix(&[
        "reset",
        "--orphans",
        dir.path().to_str().unwrap(),
        "--yes",
        "--store",
        store_path.to_str().unwrap(),
    ]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("No orphaned progress to reset."),
        "the unstamped deck's live card was judged an orphan: {}",
        stdout(&out)
    );
    let after = std::fs::read_to_string(&progress).unwrap();
    assert!(after.contains("card-math1"), "live card pruned: {after}");
}

#[test]
fn reset_orphans_on_a_deck_file_spares_a_neighbours_live_progress() {
    // A deck file names one deck, so only that deck's document is judged: a
    // neighbour sharing the store root is not in the scan, and its live
    // progress is not an orphan.
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("state");
    let target = write(dir.path(), "math.md", VALID_DECK);
    let neighbour = write(
        dir.path(),
        "other.md",
        "---\nformat-version: 1\nid: \"deck-otherdeck\"\n---\n## other <!-- id: card-other1 -->\nb\n",
    );

    let mut store = deck_store(&target, &store_path);
    store.get_or_insert("card-math1", 0);
    store.get_or_insert("orphan1", 0);
    store.save().unwrap();
    let mut neighbour_store = deck_store(&neighbour, &store_path);
    neighbour_store.get_or_insert("card-other1", 0);
    neighbour_store.save().unwrap();

    let out = alix(&[
        "reset",
        "--orphans",
        &target,
        "--yes",
        "--store",
        store_path.to_str().unwrap(),
    ]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("Reset 1 orphaned key(s)."),
        "the deck file's own orphan was not cleared: {}",
        stdout(&out)
    );
    let after = std::fs::read_to_string(store.path()).unwrap();
    assert!(!after.contains("orphan1"), "orphan not cleared: {after}");
    assert!(
        after.contains("card-math1"),
        "the target's live card was pruned: {after}"
    );
    let neighbour_after = std::fs::read_to_string(neighbour_store.path()).unwrap();
    assert!(
        neighbour_after.contains("card-other1"),
        "a neighbouring deck's live progress was pruned: {neighbour_after}"
    );
}

#[test]
fn reset_orphans_names_a_target_that_is_neither_a_deck_file_nor_a_folder() {
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("state");
    let dangling = dir.path().join("dangling.md");
    std::os::unix::fs::symlink(dir.path().join("gone.md"), &dangling).unwrap();

    for (shape, target) in [
        ("a path that does not exist", dir.path().join("missing.md")),
        ("a symlink to a deleted deck", dangling),
    ] {
        let out = alix(&[
            "reset",
            "--orphans",
            target.to_str().unwrap(),
            "--yes",
            "--store",
            store_path.to_str().unwrap(),
        ]);

        assert!(
            !out.status.success(),
            "{shape}: the sweep ran anyway: {}",
            stdout(&out)
        );
        assert!(
            stderr(&out).contains("is neither a deck file nor a folder"),
            "{shape}: refused for the wrong reason: {}",
            stderr(&out)
        );
    }
}

#[test]
fn deck_reset_drops_that_decks_virtual_cards() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let other = write(
        dir.path(),
        "other.md",
        "---\nformat-version: 1\nid: deck-otherdeck\n---\n## Other <!-- id: card-other1 -->\nanswer\n",
    );
    let store_path = dir.path().join("state");

    let mut store = deck_store(&deck, &store_path);
    let math_vc = sample_virtual_card("deck-mathdeck");
    let math_id = math_vc.id.clone();
    store.insert_virtual(math_vc);
    store.save().unwrap();
    let mut other_store = deck_store(&other, &store_path);
    let other_vc = sample_virtual_card("deck-otherdeck");
    let other_id = other_vc.id.clone();
    other_store.insert_virtual(other_vc);
    other_store.save().unwrap();

    let out = alix(&[
        "reset",
        &deck,
        "--yes",
        "--store",
        store_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let reloaded = deck_store(&deck, &store_path);
    assert!(
        reloaded.get_virtual(&math_id).is_none(),
        "the reset deck's own virtual card should be dropped"
    );
    assert!(
        deck_store(&other, &store_path)
            .get_virtual(&other_id)
            .is_some(),
        "another deck's virtual card should survive"
    );
}

#[test]
fn deck_reset_without_yes_leaves_store_unchanged() {
    // A declined/failed confirmation must not partially apply the reset: the
    // deck's mastered flag, its virtual card, and its authored progress must
    // all still be there afterwards, byte-for-byte.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let store_path = dir.path().join("state");

    let card_id = alix::deck::Deck::load(&deck).unwrap().cards[0]
        .id()
        .unwrap();
    let mut store = deck_store(&deck, &store_path);
    store.get_or_insert(&card_id, 0);
    store.set_deck_mastered("deck-mathdeck", 0);
    store.insert_virtual(sample_virtual_card("deck-mathdeck"));
    store.save().unwrap();
    let before = std::fs::read_to_string(store.path()).unwrap();

    // No `--yes` and no TTY in the test subprocess: the command must error.
    let out = alix(&["reset", &deck, "--store", store_path.to_str().unwrap()]);
    assert!(
        !out.status.success(),
        "a no-TTY reset without --yes should error"
    );

    let after = std::fs::read_to_string(store.path()).unwrap();
    assert_eq!(
        before, after,
        "the store on disk must be untouched by a declined/failed reset"
    );
    let reloaded = deck_store(&deck, &store_path);
    assert!(
        reloaded.deck_mastered("deck-mathdeck"),
        "mastered flag wiped"
    );
    assert!(reloaded.get(&card_id).is_some(), "authored progress wiped");
    assert_eq!(
        1,
        reloaded.virtual_cards_for("deck-mathdeck").len(),
        "virtual card wiped"
    );
}

#[test]
fn deck_reset_clears_a_mastery_only_store() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let state_root = dir.path().join("state");
    let mut store = deck_store(&deck, &state_root);
    store.set_deck_mastered("deck-mathdeck", alix::time::now_ms());
    store.save().unwrap();

    let out = alix(&[
        "reset",
        &deck,
        "--yes",
        "--store",
        state_root.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("Reset 0 card(s)."),
        "stdout: {}",
        stdout(&out)
    );
    assert!(
        !deck_store(&deck, &state_root).deck_mastered("deck-mathdeck"),
        "the mastery marker survived the reset"
    );
}

#[test]
fn targeted_reset_without_confirmation_names_the_card_and_preserves_it() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let state_root = dir.path().join("state");
    let mut store = deck_store(&deck, &state_root);
    store.get_or_insert("card-math1", alix::time::now_ms());
    store.save().unwrap();

    let out = alix(&[
        "reset",
        &deck,
        "--card",
        "2 + 2",
        "--store",
        state_root.to_str().unwrap(),
    ]);
    assert!(!out.status.success(), "stdout: {}", stdout(&out));
    assert!(
        stderr(&out).contains("Reset progress for What is 2 + 2?"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        deck_store(&deck, &state_root).get("card-math1").is_some(),
        "the unconfirmed targeted reset removed the card"
    );
}

#[test]
fn confirmed_virtual_only_deck_reset_clears_virtual() {
    // A deck with ONLY a virtual card (no authored progress, not mastered) must
    // still have that virtual card cleared and persisted on a confirmed reset.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let store_path = dir.path().join("state");

    let mut store = deck_store(&deck, &store_path);
    store.insert_virtual(sample_virtual_card("deck-mathdeck"));
    store.save().unwrap();

    let out = alix(&[
        "reset",
        &deck,
        "--yes",
        "--store",
        store_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let reloaded = deck_store(&deck, &store_path);
    assert_eq!(0, reloaded.virtual_cards_for("deck-mathdeck").len());
}

#[test]
fn an_unsupported_progress_document_version_is_rejected() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let state_root = dir.path().join("state");
    let store = write_progress_document(&state_root, "deck-mathdeck", "math.md", "");
    let text =
        std::fs::read_to_string(&store)
            .unwrap()
            .replacen("\"version\":1", "\"version\":999", 1);
    std::fs::write(&store, text).unwrap();

    let out = alix(&["stats", &deck, "--store", state_root.to_str().unwrap()]);
    assert!(!out.status.success());
}

#[test]
fn a_corrupt_progress_document_fails_without_overwriting_it() {
    // A damaged document must not be silently replaced with an empty one; the
    // command fails and the bytes on disk are preserved for recovery.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let garbage = "{ this is not valid json";
    let state_root = dir.path().join("state");
    let store = alix::state::UserFiles::new(&state_root).progress_for("deck-mathdeck");
    std::fs::create_dir_all(store.parent().unwrap()).unwrap();
    std::fs::write(&store, garbage).unwrap();

    let out = alix(&["stats", &deck, "--store", state_root.to_str().unwrap()]);
    assert!(
        !out.status.success(),
        "a corrupt store should fail the command"
    );
    assert_eq!(garbage, std::fs::read_to_string(&store).unwrap());
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

fn fake_reviewing_claude(dir: &Path) -> (String, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let draft = dir.join("draft-reply.txt");
    let reviewed = dir.join("reviewed-reply.txt");
    let marker = dir.join("review-marker");
    let calls = dir.join("review-calls.txt");
    std::fs::write(&draft, "## Draft question\nDraft answer\n").unwrap();
    std::fs::write(&reviewed, "## Reviewed question\nReviewed answer\n").unwrap();
    let script = dir.join("fake-reviewing-claude");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncat >/dev/null\nprintf 'x\\n' >> \"{calls}\"\n\
             if test -e \"{marker}\"; then cat \"{reviewed}\"; \
             else : > \"{marker}\"; cat \"{draft}\"; fi\n",
            calls = calls.display(),
            marker = marker.display(),
            reviewed = reviewed.display(),
            draft = draft.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (script.display().to_string(), calls)
}

#[test]
fn augment_target_format_caches_a_reshape() {
    // `deck augment --target format` reshapes a badly-shaped plain card and
    // writes the result to the deck's augmentation document, never rewriting
    // the card text. The deck is already initialized; augment open only
    // maintains missing card ids. The Claude call is faked by a config-wired
    // CLI.
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "parts.md",
        "---\nformat-version: 1\nid: \"deck-parts\"\n---\n## List the parts <!-- id: card-parts1 -->\nA, B, C\n",
    );
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
    let store = dir.path().join("state");
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

    // The reshape is cached in the state root, not written back into the deck.
    let cached = augmentation_text(&deck);
    assert!(cached.contains("\"A\""), "augmentation: {cached}");
    assert!(cached.contains("LineByLine"), "augmentation: {cached}");
    // The card's own text and token are untouched (format is display-only). The
    // deck identity remains the initialized identity.
    let deck_after = std::fs::read_to_string(&deck).unwrap();
    assert!(
        deck_after.contains("## List the parts <!-- id: card-parts1 -->\nA, B, C\n"),
        "card text and token preserved: {deck_after}"
    );
    assert!(
        deck_after.starts_with("---\nformat-version: 1\nid: \"deck-"),
        "the deck keeps its initialized frontmatter id: {deck_after}"
    );
}

#[test]
fn augment_target_format_also_covers_a_decks_virtual_card() {
    // A deck's synthesized virtual (remediation) cards get the same format
    // treatment as its authored ones — `set_format` keys by the synth card's
    // real `Card::id`, so a re-drilled remediation card is reshaped too.
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "parts.md",
        "---\nformat-version: 1\nid: \"deck-parts\"\n---\n## List the parts <!-- id: card-parts1 -->\nA, B, C\n",
    );

    let store_path = dir.path().join("state");
    let mut store = deck_store(&deck, &store_path);
    let vc = sample_virtual_card("deck-parts");
    let virtual_id = vc.id.clone();
    store.insert_virtual(vc);
    store.save().unwrap();

    // The deck's one plain card is warmed at index 0; the deck's one virtual
    // card follows it at index 1.
    let cli = fake_claude(dir.path(), r#"{"1": {"back": ["X", "Y"], "mode": "line"}}"#);
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let out = alix(&[
        "deck",
        "augment",
        &deck,
        "--target",
        "format",
        "--store",
        store_path.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let cached = augmentation_text(&deck);
    assert!(
        cached.contains(virtual_id.as_str()),
        "augmentation should key a format entry by the virtual card's id: {cached}"
    );
    assert!(cached.contains("\"X\""), "augmentation: {cached}");
}

#[test]
fn augment_target_format_skips_an_orphaned_virtual_card_colliding_with_a_real_deck_card() {
    // A partial cloze promote can leave an orphaned sidecar `virtual_cards`
    // entry whose id collides with a real deck card (see `promote_virtual`).
    // `deck augment --target format` must filter those out exactly like
    // `assemble::select`'s injection does, or the same card gets warmed twice.
    let dir = TempDir::new().unwrap();
    let deck_text = "---\nformat-version: 1\nid: \"deck-parts\"\n---\n## List the parts <!-- id: card-parts1 -->\nA, B, C\n";
    let deck = write(dir.path(), "parts.md", deck_text);
    let real_id = alix::parser::parse_str("parts.md", deck_text).unwrap()[0]
        .id()
        .unwrap();

    let store_path = dir.path().join("state");
    let mut store = deck_store(&deck, &store_path);
    store.insert_virtual(alix::store::VirtualCard {
        id: real_id.clone(), // collides with the real deck card's id — simulates an orphan
        kind: alix::store::VirtualKind::Remediation,
        deck: "deck-parts".to_string(),
        // Must reproduce `real_id` when parsed (`synthesize_virtual` matches by
        // id), so an orphan left behind by a partial promote uses the same text
        // as the now-real deck card it collides with.
        text: deck_text.to_string(),
        created_ms: 0,
    });
    store.save().unwrap();

    // Only one item should ever be warmed (the real deck card) — if the orphan
    // isn't filtered, the fake reply's index 1 lookup would matter too, but
    // asserting "1 of 1" below is what actually pins the count down.
    let cli = fake_claude(
        dir.path(),
        r#"{"0": {"back": ["A", "B", "C"], "mode": "line"}}"#,
    );
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let out = alix(&[
        "deck",
        "augment",
        &deck,
        "--target",
        "format",
        "--store",
        store_path.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("augmented 1 of 1 cards"),
        "the colliding orphan must not be double-counted alongside the real card: {}",
        stdout(&out)
    );
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
    let out = alix(&["generate", src, "--config", &config, "--print"]);
    let err = stderr(&out);
    assert!(!out.status.success(), "should fail without --yes: {err}");
    // The error must name the guard condition and point at the fix.
    assert!(err.contains("--yes"), "error must mention --yes: {err}");
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
    let out = alix(&["generate", src, "--yes", "--config", &config, "--print"]);
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
    // Guard against vacuous passes (this test once kept invoking a deleted
    // `deck generate` spelling): the run must get as far as the explore pass.
    assert!(
        err.contains("Exploring") || err.contains("is it installed"),
        "should reach the exploration/backend: {err}"
    );
}

#[test]
fn a_populated_workspace_no_longer_blocks_the_build() {
    // A populated `--workspace` used to bail before exploring (added, then
    // reverted the same day, once staging-then-merge landed): the build now
    // always stages and merges, so a populated destination must never stop
    // the run before it even reaches exploration/the backend.
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    write(&src, "notes.md", "# some source material\n");
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    write(&ws, "existing.md", "## q\na\n");
    let config = write(
        dir.path(),
        "config.toml",
        "[ask]\ncommand = \"/nonexistent/claude-xyz\"\ntimeout_secs = 5\n",
    );
    let out = alix(&[
        "generate",
        src.to_str().unwrap(),
        "--workspace",
        ws.to_str().unwrap(),
        "--config",
        &config,
    ]);
    let err = stderr(&out);
    assert!(
        !err.contains("already has files"),
        "the populated-dest guard is gone: {err}"
    );
    assert!(
        err.contains("is it installed") || err.contains("nonexistent"),
        "should get past the destination check to the exploration/backend failure: {err}"
    );
}

#[test]
fn a_leftover_staging_dir_blocks_a_headless_rebuild_until_confirmed() {
    // A staging dir kept from a previous build's merge conflicts holds the
    // only copy of that content — a rebuild must ask before wiping it, and
    // ask before spending on exploration, not after.
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    write(&src, "notes.md", "# some source material\n");
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let staging = dir.path().join(".ws.building");
    std::fs::create_dir_all(&staging).unwrap();
    write(&staging, "orphan.md", "## q\na\n");
    let config = write(
        dir.path(),
        "config.toml",
        "[ask]\ncommand = \"/nonexistent/claude-xyz\"\ntimeout_secs = 5\n",
    );

    // Without --yes: headless (no TTY) — confirm bails before any exploration.
    let out = alix(&[
        "generate",
        src.to_str().unwrap(),
        "--workspace",
        ws.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(!out.status.success(), "should fail without confirmation");
    let err = stderr(&out);
    assert!(
        err.contains(staging.to_str().unwrap())
            || err.contains("holds files from a previous build"),
        "should mention the staging path or the confirm question: {err}"
    );
    assert!(
        !err.contains("Exploring"),
        "should bail before spending on exploration: {err}"
    );
    assert!(
        staging.is_dir(),
        "declining must leave the staging dir alone"
    );

    // With --yes: the staging confirm is skipped, so the run reaches the
    // (fake) backend failure.
    let out = alix(&[
        "generate",
        src.to_str().unwrap(),
        "--workspace",
        ws.to_str().unwrap(),
        "--config",
        &config,
        "--yes",
    ]);
    let err = stderr(&out);
    assert!(
        err.contains("is it installed") || err.contains("nonexistent"),
        "should get past the staging confirm to the exploration/backend failure: {err}"
    );
}

#[test]
fn deck_check_validates_like_the_old_check() {
    // `alix deck check <deck>` must parse and report cards exactly as the old
    // `alix check <deck>` did.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let out = alix(&["doctor", &deck]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("1 cards"), "stdout: {}", stdout(&out));
}

#[test]
fn bare_check_is_gone() {
    // After the move, `alix check <deck>` is no longer a valid subcommand.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let out = alix(&["check", &deck]);
    assert!(
        !out.status.success(),
        "the old `alix check` must be gone (clap should error)"
    );
}

#[test]
fn doctor_backends_reports_a_missing_backend() {
    // Pointing `[ask] command` at a nonexistent binary → the health probe
    // reports a not-installed message and the command exits with failure.
    let dir = TempDir::new().unwrap();
    let config = write(
        dir.path(),
        "config.toml",
        "[ask]\ncommand = \"/nonexistent/no-such-cli\"\ntimeout_secs = 5\n",
    );
    let out = alix(&["doctor", "--backends", "--config", &config]);
    assert!(
        !out.status.success(),
        "a missing backend must exit with failure"
    );
    let combined = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        combined.contains("installed") || combined.contains("not found") || combined.contains('✗'),
        "output should report the failure: {combined}"
    );
}

#[test]
fn doctor_backends_reports_a_working_backend() {
    // A fake CLI that drains stdin and prints a reply → the probe reports ✓.
    let dir = TempDir::new().unwrap();
    let cli = fake_claude(dir.path(), "OK");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let out = alix(&["doctor", "--backends", "--config", &config]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let combined = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        combined.contains('✓') || combined.contains("ready") || combined.contains("ok"),
        "output should report success: {combined}"
    );
}

#[test]
fn doctor_all_backends_probes_each() {
    // `--all` probes all four backends and prints a line per backend.  All
    // will fail (none are installed in CI), but there must be output for each.
    let dir = TempDir::new().unwrap();
    let config = write(dir.path(), "config.toml", "[ask]\ntimeout_secs = 5\n");
    let out = alix(&["doctor", "--all-backends", "--config", &config]);
    // --all always exits with the overall status but must produce output for
    // each of the four backends.
    let combined = format!("{}{}", stdout(&out), stderr(&out));
    for name in ["claude", "gemini", "codex", "copilot"] {
        assert!(
            combined.contains(name),
            "output must mention '{name}': {combined}"
        );
    }
}

/// A store JSON fragment for one card: a Recall schedule in FSRS state 2
/// (`review`) due in the past, a Reconstruct schedule in state 1 (`learning`)
/// also past-due, and a set `recognized_ms`.
fn both_depths_due_card(card_id: &str) -> String {
    format!(
        r#""{card_id}":{{"acquired_ms":1000,"recall":{{"stability":10.0,"difficulty":5.0,"reps":5,"lapses":0,"state":2,"scheduled_days":20,"last_review_ms":1000,"due_ms":2000,"learning_goods":2}},"reconstruct":{{"stability":8.0,"difficulty":5.0,"reps":3,"lapses":0,"state":1,"scheduled_days":10,"last_review_ms":1000,"due_ms":2000,"learning_goods":1}},"recognized_ms":1000,"total_reviews":5,"total_passes":5}}"#
    )
}

#[test]
fn list_shows_per_depth_labels_and_recognized_mark() {
    let dir = TempDir::new().unwrap();
    // Card 1: recall=review (state 2), reconstruct=learning (state 1), recognized.
    // Card 2: recall=learning only — no reconstruct schedule, not recognized.
    let deck_text = "---\nformat-version: 1\nid: deck-cardsdeck\n---\n## Q1 <!-- id: card-q1 -->\nA1\n\n## Q2 <!-- id: card-q2 -->\nA2\n";
    let deck = write(dir.path(), "cards.md", deck_text);
    let cards = alix::parser::parse_str("cards.md", deck_text).unwrap();
    let (id1, id2) = (cards[0].id().unwrap(), cards[1].id().unwrap());
    let card1 = both_depths_due_card(&id1);
    let card2 = format!(
        r#""{id2}":{{"acquired_ms":1000,"recall":{{"stability":1.0,"difficulty":5.0,"reps":1,"lapses":0,"state":1,"scheduled_days":0,"last_review_ms":1000,"due_ms":2000,"learning_goods":1}},"total_reviews":1,"total_passes":1}}"#
    );
    let state_root = dir.path().join("state");
    write_progress_document(
        &state_root,
        "deck-cardsdeck",
        "cards.md",
        &format!("{card1},{card2}"),
    );
    let out = alix(&["list", &deck, "--store", state_root.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let result = stdout(&out);
    // Slot order is recall|reconstruct — a swap would print [  learning|    review].
    assert!(
        result.contains("[    review|  learning]✓"),
        "recall slot first, then reconstruct, then the recognized mark: {result}"
    );
    // An absent schedule shows `-` in its slot; no recognized mark → space.
    assert!(
        result.contains("[  learning|         -] "),
        "a depth without a schedule shows '-': {result}"
    );
}

#[test]
fn stats_shows_per_depth_due_counts() {
    let dir = TempDir::new().unwrap();
    let deck_text =
        "---\nformat-version: 1\nid: deck-statsdeck\n---\n## Q1 <!-- id: card-q1 -->\nA1\n";
    let deck = write(dir.path(), "stats.md", deck_text);
    let card_id = alix::parser::parse_str("stats.md", deck_text).unwrap()[0]
        .id()
        .unwrap();
    let card = both_depths_due_card(&card_id);
    let state_root = dir.path().join("state");
    write_progress_document(&state_root, "deck-statsdeck", "stats.md", &card);
    let out = alix(&["stats", &deck, "--store", state_root.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let result = stdout(&out);
    assert!(
        result.contains("due now (recall):      1"),
        "the past-due recall schedule must be counted: {result}"
    );
    assert!(
        result.contains("due now (reconstruct): 1"),
        "the past-due reconstruct schedule must be counted: {result}"
    );
}

// ── common.rs: target/workspace resolution errors ───────────────────────────

#[test]
fn a_nonexistent_target_errors_neither_deck_nor_folder() {
    let dir = TempDir::new().unwrap();
    let ghost = dir.path().join("ghost");
    let out = alix(&["stats", ghost.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("neither a deck file nor a folder"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn an_empty_folder_target_errors_no_decks() {
    let dir = TempDir::new().unwrap();
    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    let out = alix(&["stats", empty.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("no decks in"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn importing_into_a_nonexistent_workspace_errors() {
    let dir = TempDir::new().unwrap();
    let tsv = write(dir.path(), "cards.tsv", "Q1\tA1\n");
    let ghost_ws = dir.path().join("ghost-ws");
    let out = alix(&[
        "deck",
        "import",
        &tsv,
        "--workspace",
        ghost_ws.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("no folder at"),
        "stderr: {}",
        stderr(&out)
    );
}

// ── the bare `alix [dir]` launcher: pre-flight error paths ──────────────────

#[test]
fn a_nonexistent_launch_dir_errors_not_a_folder() {
    let dir = TempDir::new().unwrap();
    let ghost = dir.path().join("ghost");
    let out = alix(&[ghost.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("is not a folder"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn the_launcher_reports_an_unreadable_config_path() {
    let dir = TempDir::new().unwrap();
    let bad_config = dir.path().join("nope.toml"); // deliberately never written
    let out = alix(&[
        dir.path().to_str().unwrap(),
        "--config",
        bad_config.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("cannot read config file"),
        "stderr: {}",
        stderr(&out)
    );
}

// ── `alix config` ────────────────────────────────────────────────────────────

#[test]
fn config_bare_shows_key_bindings_and_ask_settings() {
    let out = alix(&["config"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("key bindings:"), "{text}");
    assert!(text.contains("ask:"), "{text}");
    assert!(text.contains("generate:"), "{text}");
}

#[test]
fn config_init_writes_a_file_then_refuses_to_clobber_it() {
    let home = TempDir::new().unwrap();
    let out = alix_env(&["config", "--init"], home.path(), &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("wrote "), "{}", stdout(&out));

    let out2 = alix_env(&["config", "--init"], home.path(), &[]);
    assert!(
        !out2.status.success(),
        "a second --init must refuse to clobber"
    );
    assert!(
        stderr(&out2).contains("already exists"),
        "stderr: {}",
        stderr(&out2)
    );
}

// ── `alix doctor` ────────────────────────────────────────────────────────────

#[test]
fn doctor_bare_reports_config_store_and_decks_sections() {
    let out = alix(&["doctor"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = format!("{}{}", stdout(&out), stderr(&out));
    assert!(text.contains("config"), "{text}");
    assert!(text.contains("store"), "{text}");
    assert!(text.contains("decks"), "{text}");
}

#[test]
fn bare_and_rooted_doctor_share_the_in_folder_store() {
    let decks = tempfile::tempdir().unwrap();
    std::fs::write(decks.path().join("q.md"), VALID_DECK).unwrap();

    // A config pointing decks_dir at our temp folder.
    let cfg = decks.path().join("config.toml");
    std::fs::write(
        &cfg,
        format!("decks_dir = \"{}\"\n", decks.path().display()),
    )
    .unwrap();
    let cfg = cfg.to_str().unwrap();
    let dir = decks.path().to_str().unwrap();
    let in_folder = decks.path().display().to_string();

    // No DIR: the "configured setup" branch must use the in-folder store,
    // not the global platform store.
    let bare = String::from_utf8_lossy(&alix(&["doctor", "--config", cfg]).stdout).into_owned();
    assert!(bare.contains(&in_folder), "bare doctor store, got:\n{bare}");

    // Explicit root resolves to the SAME store (the gotcha is gone).
    let rooted =
        String::from_utf8_lossy(&alix(&["doctor", dir, "--config", cfg]).stdout).into_owned();
    assert!(
        rooted.contains(&in_folder),
        "rooted doctor store, got:\n{rooted}"
    );
}

#[test]
fn doctor_on_a_folder_target_scopes_to_its_own_store() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.md", VALID_DECK);
    let out = alix(&["doctor", dir.path().to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("1 decks"), "{}", stdout(&out));
}

#[test]
fn doctor_reports_a_broken_config_as_a_failing_finding() {
    let dir = TempDir::new().unwrap();
    let config = write(dir.path(), "config.toml", "[review]\nfrobnicate = 1\n");
    let out = alix(&["doctor", "--config", &config]);
    assert!(!out.status.success(), "a broken config should fail doctor");
    let text = format!("{}{}", stdout(&out), stderr(&out));
    assert!(text.contains("config"), "{text}");
}

#[test]
fn doctor_surfaces_a_malformed_image_embed() {
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "marker.md",
        "## q <!-- id: card-markerq01 -->\nanswer\n![]()\n![x](oops\n",
    );
    let out = alix(&["doctor", &deck]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("malformed") && err.contains("image"), "{err}");
}

#[test]
fn doctor_warns_on_a_missing_image_referenced_by_the_embed() {
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "pic.md",
        "## pic <!-- id: card-picq01 -->\nphoto\n![](gone.png)\n",
    );
    let out = alix(&["doctor", &deck]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("missing image"), "{}", stderr(&out));
}

// ── `stats`/`list`/`reset` agree with the served root's store ───────────────

#[test]
fn stats_on_a_loose_deck_resolves_the_decks_dir_root_store_like_review_does() {
    // Bare `alix` and `alix stats` must resolve the same in-folder state root.
    let decks = tempfile::tempdir().unwrap();
    let deck = write(decks.path(), "math.md", VALID_DECK);

    let cfg = decks.path().join("config.toml");
    std::fs::write(
        &cfg,
        format!("decks_dir = \"{}\"\n", decks.path().display()),
    )
    .unwrap();
    let cfg = cfg.to_str().unwrap();

    let garbage = "{ this is not valid json";
    let progress = alix::state::UserFiles::new(decks.path()).progress_for("deck-mathdeck");
    std::fs::create_dir_all(progress.parent().unwrap()).unwrap();
    std::fs::write(&progress, garbage).unwrap();

    let out = alix(&["stats", &deck, "--config", cfg]);
    assert!(
        !out.status.success(),
        "stats must read the decks directory state root, not fall back to the \
         platform state root: stdout:\n{}",
        stdout(&out)
    );
}

#[test]
fn reset_all_clears_the_decks_dir_root_store_not_the_global_one() {
    // Same discriminator as above, against `reset --all`.
    let decks = tempfile::tempdir().unwrap();
    write(decks.path(), "math.md", VALID_DECK);

    let cfg = decks.path().join("config.toml");
    std::fs::write(
        &cfg,
        format!("decks_dir = \"{}\"\n", decks.path().display()),
    )
    .unwrap();
    let cfg = cfg.to_str().unwrap();

    let garbage = "{ this is not valid json";
    let progress = alix::state::UserFiles::new(decks.path()).progress_for("deck-mathdeck");
    std::fs::create_dir_all(progress.parent().unwrap()).unwrap();
    std::fs::write(&progress, garbage).unwrap();

    let out = alix(&["reset", "--all", "--yes", "--config", cfg]);
    assert!(
        !out.status.success(),
        "reset --all must read the decks directory state root, not the \
         platform state root: stdout:\n{}",
        stdout(&out)
    );
}

// ── `alix share` / `alix receive` ────────────────────────────────────────────

#[test]
fn share_on_a_nonexistent_path_errors() {
    let dir = TempDir::new().unwrap();
    let ghost = dir.path().join("ghost.md");
    let out = alix(&["share", ghost.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("neither a deck file nor a folder"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn share_on_a_folder_with_no_decks_errors() {
    let dir = TempDir::new().unwrap();
    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    let out = alix(&["share", empty.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("nothing to share"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn share_zip_writes_an_archive_of_a_single_deck() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let out_dir = dir.path().join("out");
    std::fs::create_dir(&out_dir).unwrap();
    let out = alix(&[
        "share",
        &deck,
        "--zip",
        "--output",
        out_dir.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(out_dir.join("math.zip").is_file());
    assert!(stdout(&out).contains("Wrote"), "{}", stdout(&out));
}

#[test]
fn share_zip_honors_an_explicit_output_file() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let output = dir.path().join("named-export.zip");

    let out = alix(&[
        "share",
        &deck,
        "--zip",
        "--output",
        output.to_str().unwrap(),
    ]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(output.is_file(), "{} was not written", output.display());
    assert!(
        stdout(&out).contains(&output.display().to_string()),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn a_single_deck_share_zip_restores_augmentation_without_progress_and_force_replaces_it() {
    let sender = TempDir::new().unwrap();
    let sender_decks = sender.path().join("decks");
    std::fs::create_dir(&sender_decks).unwrap();
    let deck = write(&sender_decks, "math.md", VALID_DECK);
    let mut progress = alix::state::open_store(Path::new(&deck), &sender_decks).unwrap();
    progress.get_or_insert("card-math1", 1);
    progress.save().unwrap();
    let mut augmentation = alix::augment::AugmentCache::open_for_deck(
        &alix::deck::Deck::load(Path::new(&deck)).unwrap(),
    )
    .unwrap();
    augmentation.set_note("card-math1", "shared note".to_string(), 9);
    augmentation.save().unwrap();
    let out_dir = sender.path().join("out");
    std::fs::create_dir(&out_dir).unwrap();

    let shared = alix_env(
        &[
            "share",
            &deck,
            "--zip",
            "--output",
            out_dir.to_str().unwrap(),
        ],
        sender.path(),
        &[],
    );
    assert!(shared.status.success(), "stderr: {}", stderr(&shared));

    let receiver = TempDir::new().unwrap();
    let zip = out_dir.join("math.zip");
    let received = alix_env(&["receive", zip.to_str().unwrap()], receiver.path(), &[]);
    assert!(received.status.success(), "stderr: {}", stderr(&received));
    let received_deck = receiver.path().join("decks/math.md");
    let received_state = receiver.path().join("decks");
    let received_progress = alix::state::open_store(&received_deck, &received_state).unwrap();
    assert!(received_progress.get("card-math1").is_none());
    let received_augmentation = alix::augment::AugmentCache::open_for_deck(
        &alix::deck::Deck::load(&received_deck).unwrap(),
    )
    .unwrap();
    assert_eq!(
        Some("shared note"),
        received_augmentation.note("card-math1", 9)
    );

    std::fs::write(
        &received_deck,
        "---\nformat-version: 1\nid: \"deck-mathdeck\"\n---\n## changed <!-- id: card-math1 -->\nlocal\n",
    )
    .unwrap();
    let mut changed_augmentation = alix::augment::AugmentCache::open_for_deck(
        &alix::deck::Deck::load(&received_deck).unwrap(),
    )
    .unwrap();
    changed_augmentation.set_note("card-math1", "local note".to_string(), 9);
    changed_augmentation.save().unwrap();

    let replaced = alix_env(
        &["receive", zip.to_str().unwrap(), "--force"],
        receiver.path(),
        &[],
    );
    assert!(replaced.status.success(), "stderr: {}", stderr(&replaced));
    assert_eq!(VALID_DECK, std::fs::read_to_string(&received_deck).unwrap());
    let received_augmentation = alix::augment::AugmentCache::open_for_deck(
        &alix::deck::Deck::load(&received_deck).unwrap(),
    )
    .unwrap();
    assert_eq!(
        Some("shared note"),
        received_augmentation.note("card-math1", 9)
    );
}

#[test]
fn share_zip_of_a_workspace_folder_strips_personal_state() {
    let dir = TempDir::new().unwrap();
    let ws = dir.path().join("eng");
    let members = ws.join("decks");
    std::fs::create_dir_all(&members).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"Eng\"\n").unwrap();
    write(
        &members,
        "a.md",
        "---\nformat-version: 1\nid: \"deck-a\"\n---\n## q\na\n",
    );
    std::fs::create_dir(ws.join("progress")).unwrap();
    write(&ws.join("progress"), "a.json", "{}");
    let out_dir = dir.path().join("out");
    std::fs::create_dir(&out_dir).unwrap();
    let out = alix(&[
        "share",
        ws.to_str().unwrap(),
        "--zip",
        "--output",
        out_dir.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let zip_path = out_dir.join("eng.zip");
    assert!(zip_path.is_file());

    let landed = dir.path().join("landed");
    alix::share::unzip_to(&zip_path, &landed).unwrap();
    assert!(landed.join("eng/decks/a.md").is_file());
    assert!(!landed.join("eng/progress").exists());
}

#[test]
fn share_without_wormhole_installed_reports_the_install_hint() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let out = alix_env(
        &["share", &deck],
        dir.path(),
        &[("PATH", "/nonexistent-empty-bin")],
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("is magic-wormhole installed?"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn receive_without_wormhole_installed_reports_the_install_hint() {
    let dir = TempDir::new().unwrap();
    let out = alix_env(
        &["receive", "7-fake-code-xyz"],
        dir.path(),
        &[("PATH", "/nonexistent-empty-bin")],
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("is magic-wormhole installed?"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn a_missing_zip_named_code_still_uses_the_wormhole_receive_path() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("7-fake-code.zip");

    let out = alix_env(
        &["receive", missing.to_str().unwrap()],
        dir.path(),
        &[("PATH", "/nonexistent-empty-bin")],
    );

    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("is magic-wormhole installed?"),
        "a .zip suffix alone must not select local unzip: {}",
        stderr(&out)
    );
}

#[test]
fn receive_a_zip_deck_lands_in_the_decks_dir() {
    let home = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    let deck = write(src.path(), "math.md", VALID_DECK);
    let zip_path = src.path().join("math.zip");
    alix::share::zip_to(Path::new(&deck), &zip_path).unwrap();

    let out = alix_env(&["receive", zip_path.to_str().unwrap()], home.path(), &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(home.path().join("decks/math.md").is_file());
    assert!(
        stdout(&out).contains("shows up in the picker"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn receive_an_existing_deck_without_force_errors_then_force_overwrites() {
    let home = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    let deck = write(src.path(), "math.md", VALID_DECK);
    let zip_path = src.path().join("math.zip");
    alix::share::zip_to(Path::new(&deck), &zip_path).unwrap();

    let first = alix_env(&["receive", zip_path.to_str().unwrap()], home.path(), &[]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));

    // The same code, received again: the deck is already there.
    let second = alix_env(&["receive", zip_path.to_str().unwrap()], home.path(), &[]);
    assert!(
        !second.status.success(),
        "should refuse to clobber without --force"
    );
    assert!(
        stderr(&second).contains("pass --force to overwrite"),
        "stderr: {}",
        stderr(&second)
    );

    let third = alix_env(
        &["receive", zip_path.to_str().unwrap(), "--force"],
        home.path(),
        &[],
    );
    assert!(third.status.success(), "stderr: {}", stderr(&third));
}

#[test]
fn receive_a_zip_folder_strips_leaked_personal_files() {
    let home = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    let ws = src.path().join("eng");
    std::fs::create_dir(&ws).unwrap();
    write(&ws, "a.md", "## q\na\n");
    std::fs::create_dir(ws.join("progress")).unwrap();
    write(&ws.join("progress"), "a.json", "{}");
    let zip_path = src.path().join("eng.zip");
    alix::share::zip_to(&ws, &zip_path).unwrap();

    let out = alix_env(&["receive", zip_path.to_str().unwrap()], home.path(), &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("stripped a leaked personal file: progress"),
        "{}",
        stdout(&out)
    );
    assert!(home.path().join("decks/eng/a.md").is_file());
    assert!(!home.path().join("decks/eng/progress").exists());
}

#[test]
fn receive_a_zip_folder_rejects_the_workspace_flag() {
    let home = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    let ws = src.path().join("eng");
    std::fs::create_dir(&ws).unwrap();
    write(&ws, "a.md", "## q\na\n");
    let zip_path = src.path().join("eng.zip");
    alix::share::zip_to(&ws, &zip_path).unwrap();

    let out = alix_env(
        &[
            "receive",
            zip_path.to_str().unwrap(),
            "--workspace",
            "/tmp/nonexistent-ws-for-alix-tests",
        ],
        home.path(),
        &[],
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("--workspace places a received deck"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn receive_a_zip_folder_refuses_to_clobber_an_existing_dest() {
    let home = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    let ws = src.path().join("eng");
    std::fs::create_dir(&ws).unwrap();
    write(&ws, "a.md", "## q\na\n");
    let zip_path = src.path().join("eng.zip");
    alix::share::zip_to(&ws, &zip_path).unwrap();

    let first = alix_env(&["receive", zip_path.to_str().unwrap()], home.path(), &[]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));

    let second = alix_env(&["receive", zip_path.to_str().unwrap()], home.path(), &[]);
    assert!(!second.status.success());
    assert!(
        stderr(&second).contains("already exists — move it aside first"),
        "stderr: {}",
        stderr(&second)
    );
}

// ── `alix generate`: trace stub / suggest / walk with a fake backend ────────

#[test]
fn generate_builds_checkpoints_into_an_existing_trace_stub() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "notes.md", "some source material\n");
    let stub = write(
        dir.path(),
        "t.md",
        "---\ntrace: how it works\nsource: .\n---\n",
    );
    let cli = fake_claude(
        dir.path(),
        "## checkpoint one\nsome point\n<!-- at: notes.md:1 -->\n",
    );
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let out = alix(&["generate", &stub, "--config", &config, "--yes"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("Wrote 1 checkpoints"),
        "{}",
        stdout(&out)
    );
    let rewritten = std::fs::read_to_string(&stub).unwrap();
    assert!(rewritten.contains("trace: how it works"), "{rewritten}");
    assert!(rewritten.contains("checkpoint one"), "{rewritten}");
}

#[test]
fn generate_refuses_to_rebuild_trace_checkpoints_without_force() {
    let dir = TempDir::new().unwrap();
    let stub = write(
        dir.path(),
        "t.md",
        "---\ntrace: how it works\nsource: .\n---\n## old checkpoint <!-- id: card-c1 -->\nold point\n<!-- at: 1 -->\n",
    );
    let original = std::fs::read_to_string(&stub).unwrap();

    let out = alix(&["generate", &stub]);

    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("already has checkpoints"),
        "stderr: {}",
        stderr(&out)
    );
    assert_eq!(original, std::fs::read_to_string(&stub).unwrap());
    assert!(!dir.path().join("t.md.bak").exists());
}

#[test]
fn generate_trace_plan_prints_the_suggestion_menu() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "notes.md", "some source material\n");
    let cli = fake_claude(dir.path(), "1. [trace] how X becomes Y\n   source: .\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let out = alix(&[
        "generate",
        dir.path().to_str().unwrap(),
        "--trace",
        "--plan",
        "--config",
        &config,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("how X becomes Y"), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("Paste a suggestion into a new deck"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn generate_trace_walk_writes_an_explore_deck() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "notes.md", "some source material\n");
    let cli = fake_claude(dir.path(), "## what it is\nsome point\n<!-- at: 1 -->\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let out_path = dir.path().join("walk.md");
    let out = alix(&[
        "generate",
        dir.path().to_str().unwrap(),
        "--trace",
        "--config",
        &config,
        "--output",
        out_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = std::fs::read_to_string(&out_path).unwrap();
    assert!(text.contains("trace: \"exploring"), "{text}");
    assert!(text.contains("source:"), "{text}");
    assert!(text.contains("what it is"), "{text}");
}

#[test]
fn generate_trace_walk_refuses_to_clobber_an_existing_output() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "notes.md", "some source material\n");
    let cli = fake_claude(dir.path(), "## what it is\nsome point\n<!-- at: 1 -->\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let out_path = dir.path().join("walk.md");
    write(dir.path(), "walk.md", "already here\n");
    let out = alix(&[
        "generate",
        dir.path().to_str().unwrap(),
        "--trace",
        "--config",
        &config,
        "--output",
        out_path.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("already exists; pass --force to overwrite"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn ordinary_text_and_markdown_files_never_enter_trace_building() {
    for (name, source_text) in [
        (
            "trace-shaped.txt",
            "---\ntrace: how it works\nsource: .\n---\n",
        ),
        (
            "ordinary.md",
            "---\nformat-version: 1\nid: deck-source\n---\n## Source question <!-- id: card-source1 -->\nSource answer\n",
        ),
    ] {
        let dir = TempDir::new().unwrap();
        let source = write(dir.path(), name, source_text);
        let cli = fake_claude(dir.path(), "## Generated question\nGenerated answer\n");
        let config = write(
            dir.path(),
            "config.toml",
            &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
        );

        let out = alix(&["generate", &source, "--config", &config, "--print"]);

        assert!(out.status.success(), "{name}: {}", stderr(&out));
        assert_eq!(
            "## Generated question\nGenerated answer\n",
            stdout(&out),
            "{name}"
        );
        assert_eq!(source_text, std::fs::read_to_string(source).unwrap());
    }
}

// ── `alix generate`: a single deck from a URL/file source, fake backend ─────

#[test]
fn generate_single_deck_writes_a_deck_file() {
    let dir = TempDir::new().unwrap();
    let cli = fake_claude(dir.path(), "## Generated Q\nGenerated A\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "").unwrap();
    let out = alix(&[
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "gen",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("Wrote 1 cards to"),
        "{}",
        stdout(&out)
    );
    assert!(ws.join("decks/gen.md").is_file());
}

#[test]
fn generate_keeps_and_warns_about_a_deck_over_max_cards() {
    let dir = TempDir::new().unwrap();
    let cli = fake_claude(dir.path(), "## Q1\nA1\n\n## Q2\nA2\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n\n[generate]\nmax_cards = 1\n"),
    );
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "").unwrap();
    let out = alix(&[
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "gen",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("above the configured max_cards = 1"),
        "soft-ceiling warning missing: {}",
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("Wrote 2 cards to"),
        "all cards must be kept: {}",
        stdout(&out)
    );
}

#[test]
fn generate_does_not_warn_when_card_count_equals_the_soft_maximum() {
    let dir = TempDir::new().unwrap();
    let cli = fake_claude(dir.path(), "## Generated Q\nGenerated A\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );

    let out = alix(&[
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--cards",
        "1",
        "--print",
    ]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        !stderr(&out).contains("above the configured max_cards"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn either_review_switch_independently_runs_the_second_generation_pass() {
    for (cli_review, config_review) in [(true, false), (false, true)] {
        let dir = TempDir::new().unwrap();
        let (cli, calls) = fake_reviewing_claude(dir.path());
        let config = write(
            dir.path(),
            "config.toml",
            &format!(
                "[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n\
                 [generate]\nreview = {config_review}\n"
            ),
        );
        let mut args = vec![
            "generate",
            "https://example.org/page",
            "--config",
            &config,
            "--print",
        ];
        if cli_review {
            args.push("--review");
        }

        let out = alix(&args);

        assert!(
            out.status.success(),
            "cli={cli_review}, config={config_review}: {}",
            stderr(&out)
        );
        assert_eq!(
            "## Reviewed question\nReviewed answer\n",
            stdout(&out),
            "cli={cli_review}, config={config_review}"
        );
        assert_eq!(
            2,
            std::fs::read_to_string(&calls).unwrap().lines().count(),
            "cli={cli_review}, config={config_review}"
        );
    }
}

#[test]
fn generate_print_normalizes_exactly_one_trailing_newline() {
    for reply in [
        "## Generated Q\nGenerated A",
        "## Generated Q\nGenerated A\n",
    ] {
        let dir = TempDir::new().unwrap();
        let cli = fake_claude(dir.path(), reply);
        let config = write(
            dir.path(),
            "config.toml",
            &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
        );

        let out = alix(&[
            "generate",
            "https://example.org/page",
            "--config",
            &config,
            "--print",
        ]);

        assert!(out.status.success(), "stderr: {}", stderr(&out));
        assert_eq!("## Generated Q\nGenerated A\n", stdout(&out));
    }
}

#[test]
fn generate_over_an_existing_deck_replaces_it_instead_of_writing_a_new_one() {
    let dir = TempDir::new().unwrap();
    let cli = fake_claude(dir.path(), "## Generated Q\nGenerated A\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "").unwrap();
    let args = |extra: &[&str]| {
        let mut a = vec![
            "generate".to_string(),
            "https://example.org/page".to_string(),
            "--config".to_string(),
            config.clone(),
            "--workspace".to_string(),
            ws.to_str().unwrap().to_string(),
            "--output".to_string(),
            "gen".to_string(),
        ];
        a.extend(extra.iter().map(|s| s.to_string()));
        a
    };

    let first = alix(&args(&[]).iter().map(String::as_str).collect::<Vec<_>>());
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    assert!(stdout(&first).contains("Wrote"), "{}", stdout(&first));

    // The destination now exists, so the second run takes the replace path:
    // it rewrites the deck in place and reports wiped progress, rather than
    // placing a fresh one.
    let second = alix(
        &args(&["--force"])
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    assert!(
        stdout(&second).contains("Replaced"),
        "an existing destination must be replaced, not written anew: {}",
        stdout(&second)
    );
    assert!(ws.join("decks/gen.md").is_file());
}

#[test]
fn generate_forwards_structured_agent_progress_without_printing_partial_markdown() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let cli = dir.path().join("fake-claude");
    std::fs::write(
        &cli,
        r###"#!/bin/sh
cat >/dev/null
printf '%s\n' '{"type":"system","subtype":"init"}'
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"WebFetch","input":{"url":"https://example.org"}}]}}'
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"partial deck must stay hidden"}]}}'
printf '%s\n' '{"type":"result","subtype":"success","result":"## Generated Q\nGenerated A\n"}'
"###,
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{}\"\n", cli.display()),
    );

    let out = alix(&[
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--print",
    ]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("Claude: fetching the source..."),
        "stderr: {}",
        stderr(&out)
    );
    assert!(!stderr(&out).contains("partial deck must stay hidden"));
    assert!(stdout(&out).contains("## Generated Q\nGenerated A"));
}

#[test]
fn generate_single_deck_passes_goal_language_and_card_style_to_the_model() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let prompt = dir.path().join("prompt.txt");
    let reply = dir.path().join("reply.txt");
    std::fs::write(
        &reply,
        "## Welche Stadt ist die Hauptstadt?\n- [ ] Hamburg\n- [x] Berlin\n- [ ] München\n",
    )
    .unwrap();
    let cli = dir.path().join("fake-claude");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\ncat > {}\ncat {}\n",
            prompt.display(),
            reply.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = write(
        dir.path(),
        "config.toml",
        &format!(
            "[ask]\ncommand = \"{}\"\ntimeout_secs = 10\n",
            cli.display()
        ),
    );

    let out = alix(&[
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--goal",
        "recognize Germany's institutions",
        "--language",
        "German",
        "--audience",
        "new voters",
        "--card-style",
        "authored-choices",
        "--print",
    ]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let prompt = std::fs::read_to_string(prompt).unwrap();
    assert!(prompt.contains("recognize Germany's institutions"));
    assert!(prompt.contains("German"));
    assert!(prompt.contains("new voters"));
    assert!(prompt.contains("authored multiple-choice"));
}

#[test]
fn generate_workspace_applies_goal_language_audience_and_card_style() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    write(&source, "notes.md", "Berlin is Germany's capital.\n");

    let plan = write(
        dir.path(),
        "plan.txt",
        "Goal    recognize institutions\n\
         Source  notes\n\
         Spine   basics\n\n\
         1. [deck] Institutions\n\
            requires: none\n\
            @source: notes.md\n\
         2. [deck] Terms\n\
            requires: 1\n\
            @source: notes.md\n",
    );
    let filled = write(
        dir.path(),
        "filled.txt",
        "=== item 1 ===\n\
         ## Welche Stadt ist die Hauptstadt?\n\
         - [ ] Hamburg\n\
         - [x] Berlin\n\
         - [ ] München\n\
         > Hamburg und München sind Großstädte, aber keine Bundeshauptstadt.\n\
         <!-- at: notes.md:1 -->\n\
         === item 2 ===\n\
         ## Welche Ebene ist hier gemeint?\n\
         - [ ] Kommune\n\
         - [x] Bund\n\
         - [ ] Land\n\
         > Kommune und Land bezeichnen andere staatliche Ebenen.\n\
         <!-- at: notes.md:1 -->\n",
    );
    let request = dir.path().join("request.txt");
    let cli = dir.path().join("fake-claude");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\ncat > {request}\n\
             if grep -q 'Now WRITE THE FULL CONTENT' {request}; then\n\
               cat {filled}\n\
             else\n\
               cat {plan}\n\
             fi\n",
            request = request.display(),
            filled = filled,
            plan = plan,
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = write(
        dir.path(),
        "config.toml",
        &format!(
            "[ask]\ncommand = \"{}\"\ntimeout_secs = 10\n",
            cli.display()
        ),
    );
    let icon = write(
        dir.path(),
        "icon.svg",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1 1\"></svg>\n",
    );
    let workspace = dir.path().join("workspace");

    let out = alix(&[
        "generate",
        source.to_str().unwrap(),
        "--config",
        &config,
        "--workspace",
        workspace.to_str().unwrap(),
        "--icon",
        &icon,
        "--goal",
        "recognize Germany's institutions",
        "--language",
        "German",
        "--audience",
        "new voters",
        "--card-style",
        "authored-choices",
        "--yes",
    ]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("Claude: producing a response..."),
        "workspace generation should report wrapper activity: {}",
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("2 filled, 0 stub(s) (0 traces, 2 decks)."),
        "stdout: {}",
        stdout(&out)
    );
    assert!(
        stdout(&out).contains("source and image asset(s) for 2 deck(s)"),
        "stdout: {}",
        stdout(&out)
    );
    let request = std::fs::read_to_string(request).unwrap();
    assert!(request.contains("recognize Germany's institutions"));
    assert!(request.contains("German"));
    assert!(request.contains("new voters"));
    assert!(request.contains("authored multiple-choice"));
    for name in ["01-institutions.md", "02-terms.md"] {
        let deck = alix::deck::Deck::load(workspace.join("decks").join(name)).unwrap();
        assert!(
            deck.cards
                .iter()
                .all(|card| card.authored_distractors.len() == 2),
            "{name} should contain only authored three-option cards"
        );
    }
}

#[test]
fn generated_workspace_reports_one_stub_and_stays_silent_at_zero_frozen_assets() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    write(&source, "notes.md", "local exploration seed\n");
    let plan = write(
        dir.path(),
        "plan.txt",
        "Goal    learn two topics\n\
         Source  public material\n\
         Spine   first to second\n\n\
         1. [deck] First\n\
            requires: none\n\
            @source: https://example.org/first\n\
         2. [deck] Second\n\
            requires: 1\n\
            @source: https://example.org/second\n",
    );
    let filled = write(
        dir.path(),
        "filled.txt",
        "=== item 1 ===\n## First question\nFirst answer\n",
    );
    let request = dir.path().join("request.txt");
    let cli = dir.path().join("fake-claude");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\ncat > \"{request}\"\n\
             if grep -q 'Now WRITE THE FULL CONTENT' \"{request}\"; then \
             cat \"{filled}\"; else cat \"{plan}\"; fi\n",
            request = request.display(),
            filled = filled,
            plan = plan,
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = write(
        dir.path(),
        "config.toml",
        &format!(
            "[ask]\ncommand = \"{}\"\ntimeout_secs = 10\n",
            cli.display()
        ),
    );
    let icon = write(
        dir.path(),
        "icon.svg",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1 1\"></svg>\n",
    );
    let workspace = dir.path().join("workspace");

    let out = alix(&[
        "generate",
        source.to_str().unwrap(),
        "--config",
        &config,
        "--workspace",
        workspace.to_str().unwrap(),
        "--icon",
        &icon,
        "--yes",
    ]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("1 filled, 1 stub(s) (0 traces, 2 decks)."),
        "stdout: {}",
        stdout(&out)
    );
    assert!(
        !stdout(&out).contains("Froze "),
        "zero frozen assets must not print a summary: {}",
        stdout(&out)
    );
}

#[test]
fn generate_single_deck_records_the_explicit_public_url_as_a_source() {
    let dir = TempDir::new().unwrap();
    let cli = fake_claude(
        dir.path(),
        "---\nlink: https://mirror.example/page\nsource: https://mirror.example/page\n---\n\n## Generated Q\nGenerated A\n",
    );
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "").unwrap();

    let out = alix(&[
        "generate",
        "https://mirror.example/page",
        "--source-url",
        "https://canonical.example/page",
        "--config",
        &config,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "gen",
    ]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let deck = std::fs::read_to_string(ws.join("decks/gen.md")).unwrap();
    assert!(
        deck.contains(
            "source:\n  - \"https://mirror.example/page\"\n  - \"https://canonical.example/page\""
        ),
        "{deck}"
    );
    assert!(!deck.contains("origin"), "{deck}");
}

#[test]
fn generate_single_deck_print_flag_prints_without_writing() {
    let dir = TempDir::new().unwrap();
    let cli = fake_claude(dir.path(), "## Generated Q\nGenerated A\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let out = alix(&[
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--print",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("Generated Q"), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("cards, not written; --print"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn generate_single_deck_refuses_to_clobber_without_force_then_force_overwrites() {
    let dir = TempDir::new().unwrap();
    let cli = fake_claude(dir.path(), "## Generated Q\nGenerated A\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "").unwrap();
    let args = [
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "gen",
    ];
    let first = alix(&args);
    assert!(first.status.success(), "stderr: {}", stderr(&first));

    let second = alix(&args);
    assert!(!second.status.success());
    assert!(
        stderr(&second).contains("already exists; pass --force to overwrite"),
        "stderr: {}",
        stderr(&second)
    );

    let mut forced = args.to_vec();
    forced.push("--force");
    let third = alix(&forced);
    assert!(third.status.success(), "stderr: {}", stderr(&third));
}

#[test]
fn generate_single_deck_rejects_invalid_math_without_touching_the_destination() {
    let dir = TempDir::new().unwrap();
    let cli = fake_claude(dir.path(), "## Generated Q\n$\\frac{1$\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "").unwrap();
    let target = write(&ws.join("decks"), "gen.md", "original bytes\n");
    let out = alix(&[
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "gen",
        "--force",
    ]);

    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("invalid LaTeX math"),
        "stderr: {}",
        stderr(&out)
    );
    assert_eq!("original bytes\n", std::fs::read_to_string(target).unwrap());
    assert_eq!(1, std::fs::read_dir(ws.join("decks")).unwrap().count());
}

#[test]
fn generate_single_deck_invalid_math_creates_no_new_destination() {
    let dir = TempDir::new().unwrap();
    let cli = fake_claude(dir.path(), "## Generated Q\n$\\frac{1$\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "").unwrap();
    let out = alix(&[
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "gen",
    ]);

    assert!(!out.status.success());
    assert!(!ws.join("decks/gen.md").exists());
}

#[test]
fn generate_single_deck_prints_invalid_math_with_a_warning_without_writing() {
    let dir = TempDir::new().unwrap();
    let cli = fake_claude(dir.path(), "## Generated Q\n$\\frac{1$\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let out = alix(&[
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--print",
    ]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("$\\frac{1$"));
    assert!(
        stderr(&out).contains("invalid LaTeX math"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn generate_single_deck_still_saves_text_that_does_not_parse() {
    let dir = TempDir::new().unwrap();
    let cli = fake_claude(dir.path(), "## missing answer\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "").unwrap();
    let out = alix(&[
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "draft",
    ]);

    assert!(!out.status.success());
    assert!(ws.join("decks/draft.md").exists());
    assert!(
        stderr(&out).contains("Saved the generated deck"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn generate_on_a_directory_source_explores_then_falls_back_to_a_single_deck() {
    use std::os::unix::fs::PermissionsExt;

    // A real one-item plan routes to a single deck rather than a multi-item
    // workspace build.
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir(&src).unwrap();
    write(&src, "notes.md", "some source material\n");
    let request = dir.path().join("request.txt");
    let plan = write(
        dir.path(),
        "plan.txt",
        "1. [deck] The facts\n   requires: none\n   @source: notes.md\n",
    );
    let deck = write(dir.path(), "generated.txt", "## Generated Q\nGenerated A\n");
    let cli = dir.path().join("fake-claude");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\ncat > {request}\n\
             if grep -q 'Output a PLAN' {request}; then\n\
               cat {plan}\n\
             else\n\
               cat {deck}\n\
             fi\n",
            request = request.display(),
            plan = plan,
            deck = deck,
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = write(
        dir.path(),
        "config.toml",
        &format!(
            "[ask]\ncommand = \"{}\"\ntimeout_secs = 10\n",
            cli.display()
        ),
    );
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "").unwrap();
    let out = alix(&[
        "generate",
        src.to_str().unwrap(),
        "--config",
        &config,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "gen",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("Exploring"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(ws.join("decks/gen.md").is_file());
}

// ── `alix deck augment`: each target, fake backend ──────────────────────────

#[test]
fn augment_choices_caches_distractors_for_two_cards() {
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "quiz.md",
        "---\nformat-version: 1\nid: \"deck-quiz\"\n---\n## Q1 <!-- id: card-q1 -->\nA1\n\n## Q2 <!-- id: card-q2 -->\nA2\n",
    );
    let cli = fake_claude(dir.path(), r#"{"0": ["W1", "W2"], "1": ["W3", "W4"]}"#);
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let store = dir.path().join("state");
    let out = alix(&[
        "deck",
        "augment",
        &deck,
        "--target",
        "choices",
        "--store",
        store.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("augmented 2 of 2 cards"),
        "{}",
        stdout(&out)
    );
    let cached = augmentation_text(&deck);
    assert!(cached.contains("W1"), "{cached}");
    assert!(cached.contains("W3"), "{cached}");
    assert!(
        !store.join("augment").exists(),
        "--store selects user files and must not relocate workspace augmentation"
    );
}

#[test]
fn a_workspace_store_override_does_not_relocate_augmentation() {
    let dir = TempDir::new().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(workspace.join("decks")).unwrap();
    std::fs::write(
        workspace.join("alix.toml"),
        "title = \"Quiz\"\nstore = \"user-files\"\n",
    )
    .unwrap();
    let deck = write(
        &workspace.join("decks"),
        "quiz.md",
        "---\nformat-version: 1\nid: \"deck-quiz\"\n---\n## Q1 <!-- id: card-q1 -->\nA1\n",
    );
    let cli = fake_claude(dir.path(), r#"{"0": ["W1", "W2"]}"#);
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );

    let out = alix(&[
        "deck", "augment", &deck, "--target", "choices", "--config", &config,
    ]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(workspace.join("augment/deck-quiz.json").is_file());
    assert!(!workspace.join("user-files/augment").exists());
}

#[test]
fn augment_notes_caches_a_trivia_note() {
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "quiz.md",
        "---\nformat-version: 1\nid: \"deck-quiz\"\n---\n## Q1 <!-- id: card-q1 -->\nA1\n",
    );
    let cli = fake_claude(dir.path(), r#"{"0": "a fun fact"}"#);
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let store = dir.path().join("state");
    let out = alix(&[
        "deck",
        "augment",
        &deck,
        "--target",
        "notes",
        "--store",
        store.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let cached = augmentation_text(&deck);
    assert!(cached.contains("a fun fact"), "{cached}");
}

#[test]
fn augment_without_a_store_flag_caches_beside_the_loose_deck() {
    let decks = tempfile::tempdir().unwrap();
    let deck = write(
        decks.path(),
        "quiz.md",
        "---\nformat-version: 1\nid: \"deck-quiz\"\n---\n## Q1 <!-- id: card-q1 -->\nA1\n",
    );
    let cli = fake_claude(decks.path(), r#"{"0": "a fun fact"}"#);

    let cfg = decks.path().join("config.toml");
    std::fs::write(
        &cfg,
        format!(
            "decks_dir = \"{}\"\n[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n",
            decks.path().display()
        ),
    )
    .unwrap();
    let cfg = cfg.to_str().unwrap();

    let out = alix(&[
        "deck", "augment", &deck, "--target", "notes", "--config", cfg,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let cached = augmentation_text(&deck);
    assert!(cached.contains("a fun fact"), "{cached}");
}

#[test]
fn augment_questions_caches_a_reworded_variant() {
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "quiz.md",
        "---\nformat-version: 1\nid: \"deck-quiz\"\n---\n## Q1 <!-- id: card-q1 -->\nA1\n",
    );
    let cli = fake_claude(dir.path(), r#"{"0": ["Rephrased Q1?"]}"#);
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let store = dir.path().join("state");
    let out = alix(&[
        "deck",
        "augment",
        &deck,
        "--target",
        "questions",
        "--store",
        store.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let cached = augmentation_text(&deck);
    assert!(cached.contains("Rephrased Q1?"), "{cached}");
}

#[test]
fn augment_questions_on_a_cloze_only_deck_errors() {
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "c.md",
        "---\nformat-version: 1\nid: \"deck-cloze\"\n---\n## Complete <!-- id: card-c1 -->\nThe capital of France is \\blank{Paris}.\n",
    );
    let config = write(
        dir.path(),
        "config.toml",
        "[ask]\ncommand = \"/nonexistent/x\"\n",
    );
    let store = dir.path().join("state");
    let out = alix(&[
        "deck",
        "augment",
        &deck,
        "--target",
        "questions",
        "--store",
        store.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("no plain (non-cloze) cards to add question variants to"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn augment_keypoints_caches_decomposed_claims() {
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "quiz.md",
        "---\nformat-version: 1\nid: \"deck-quiz\"\n---\n## Q1 <!-- id: card-q1 -->\nA1\n",
    );
    let cli = fake_claude(dir.path(), r#"{"0": ["point one", "point two"]}"#);
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let store = dir.path().join("state");
    let out = alix(&[
        "deck",
        "augment",
        &deck,
        "--target",
        "keypoints",
        "--store",
        store.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let cached = augmentation_text(&deck);
    assert!(cached.contains("point one"), "{cached}");
    assert!(cached.contains("point two"), "{cached}");
}

#[test]
fn augment_order_prints_and_caches_the_walk() {
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "quiz.md",
        "---\nformat-version: 1\nid: \"deck-quiz\"\n---\n## Q1 <!-- id: card-q1 -->\nA1\n\n## Q2 <!-- id: card-q2 -->\nA2\n",
    );
    let cli = fake_claude(
        dir.path(),
        r#"{"principle": "by difficulty", "edges": [{"from": 0, "to": 1, "label": "builds on"}], "walk": [0, 1], "regions": []}"#,
    );
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let store = dir.path().join("state");
    let out = alix(&[
        "deck",
        "augment",
        &deck,
        "--target",
        "order",
        "--store",
        store.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("order 'pedagogical order'"), "{text}");
    assert!(text.contains("by difficulty"), "{text}");
    assert!(text.contains("(1 order stored for this deck)"), "{text}");
}

#[test]
fn augment_on_an_empty_deck_errors_without_calling_the_backend() {
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "empty.md",
        "---\nformat-version: 1\nid: deck-emptydeck\n---\n# Nothing\n",
    );
    let config = write(
        dir.path(),
        "config.toml",
        "[ask]\ncommand = \"/nonexistent/x\"\n",
    );
    let store = dir.path().join("state");
    let out = alix(&[
        "deck",
        "augment",
        &deck,
        "--target",
        "choices",
        "--store",
        store.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("the deck has no cards to augment"),
        "stderr: {}",
        stderr(&out)
    );
}

// ── `alix deck import` ───────────────────────────────────────────────────────

#[test]
fn deck_init_stamps_an_intended_markdown_deck() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "notes.md", "## Question\nAnswer\n");

    let out = alix(&["deck", "init", &deck]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("Initialized"),
        "stdout: {}",
        stdout(&out)
    );
    let stamped = std::fs::read_to_string(&deck).unwrap();
    assert!(
        stamped.starts_with("---\nformat-version: 1\nid: \"deck-"),
        "{stamped}"
    );
    assert_eq!(1, stamped.matches("<!-- id: ").count(), "{stamped}");
}

#[test]
fn deck_init_refuses_plain_prose_without_changing_it() {
    let dir = TempDir::new().unwrap();
    let original = "# Notes\n\nordinary prose\n";
    let path = write(dir.path(), "notes.md", original);

    let out = alix(&["deck", "init", &path]);

    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("is not a deck"),
        "stderr: {}",
        stderr(&out)
    );
    assert_eq!(original, std::fs::read_to_string(path).unwrap());
}

#[test]
fn deck_init_refuses_a_generic_frontmatter_id_without_changing_it() {
    let dir = TempDir::new().unwrap();
    let original = "---\nid: \"article\"\n---\n## Question\nAnswer\n";
    let path = write(dir.path(), "notes.md", original);

    let out = alix(&["deck", "init", &path]);

    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("`deck-<token>` id"),
        "stderr: {}",
        stderr(&out)
    );
    assert_eq!(original, std::fs::read_to_string(path).unwrap());
}

#[test]
fn doctor_recommends_initializing_deck_like_markdown() {
    let dir = TempDir::new().unwrap();
    let path = write(
        dir.path(),
        "notes.md",
        "# Notes\n\n## Design\nordinary prose\n",
    );

    let out = alix(&["doctor", &path]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("deck init"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn deck_import_writes_a_deck_from_tsv() {
    let dir = TempDir::new().unwrap();
    let tsv = write(
        dir.path(),
        "cards.tsv",
        "Capital of Japan?\tTokyo\nCapital of Italy?\tRome\n",
    );
    let out = alix(&["deck", "import", &tsv, "--output", "geo"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("Imported 2 cards into"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn deck_import_print_flag_prints_without_writing() {
    let dir = TempDir::new().unwrap();
    let tsv = write(dir.path(), "cards.tsv", "Q1\tA1\n");
    let out = alix(&["deck", "import", &tsv, "--print"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!("## Q1\nA1\n\n", stdout(&out));
    assert!(
        stderr(&out).contains("cards, not written; --print"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn deck_import_into_a_workspace_lands_there() {
    let dir = TempDir::new().unwrap();
    let tsv = write(dir.path(), "cards.tsv", "Q1\tA1\n");
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "").unwrap();
    let out = alix(&[
        "deck",
        "import",
        &tsv,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "geo",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(ws.join("decks/geo.md").is_file());
}

#[test]
fn deck_import_refuses_to_clobber_without_force_then_force_overwrites() {
    let dir = TempDir::new().unwrap();
    let tsv = write(dir.path(), "cards.tsv", "Q1\tA1\n");
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "").unwrap();
    let args = [
        "deck",
        "import",
        &tsv,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "geo",
    ];
    let first = alix(&args);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    let placed = std::fs::read_to_string(ws.join("decks/geo.md")).unwrap();

    let second = alix(&args);
    assert!(!second.status.success());
    assert!(
        stderr(&second).contains("already exists; pass --force to overwrite"),
        "stderr: {}",
        stderr(&second)
    );
    assert_eq!(
        placed,
        std::fs::read_to_string(ws.join("decks/geo.md")).unwrap(),
        "the deck must be untouched when --force is absent"
    );
    assert!(!ws.join("decks/geo.md.bak").exists());

    // The kept `.md.bak` proves the replace protocol ran; a plain overwrite
    // leaves none.
    let mut forced = args.to_vec();
    forced.push("--force");
    let third = alix(&forced);
    assert!(third.status.success(), "stderr: {}", stderr(&third));
    assert_eq!(
        placed,
        std::fs::read_to_string(ws.join("decks/geo.md.bak")).unwrap()
    );
}

// ── `alix deck remove` / `alix deck restore` ─────────────────────────────────

fn imported_workspace_deck(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let tsv = write(dir, "cards.tsv", "Q1\tA1\n");
    let ws = dir.join("ws");
    std::fs::create_dir_all(ws.join("decks")).unwrap();
    std::fs::write(ws.join("alix.toml"), "").unwrap();
    let imported = alix(&[
        "deck",
        "import",
        &tsv,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "geo",
    ]);
    assert!(imported.status.success(), "stderr: {}", stderr(&imported));
    let deck = ws.join("decks/geo.md");
    (ws, deck)
}

#[test]
fn deck_remove_without_yes_refuses_headless_and_touches_nothing() {
    let dir = TempDir::new().unwrap();
    let (_ws, deck) = imported_workspace_deck(dir.path());

    let out = alix(&["deck", "remove", deck.to_str().unwrap()]);

    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("refusing without a terminal"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(
        !stdout(&out).contains("warning: required by"),
        "stdout: {}",
        stdout(&out)
    );
    assert!(deck.exists(), "nothing may be removed without confirmation");
}

#[test]
fn deck_remove_with_yes_deletes_the_deck_and_its_backups() {
    let dir = TempDir::new().unwrap();
    let (ws, deck) = imported_workspace_deck(dir.path());
    let tsv2 = write(dir.path(), "cards2.tsv", "Q2\tA2\n");
    let replaced = alix(&[
        "deck",
        "import",
        &tsv2,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "geo",
        "--force",
    ]);
    assert!(replaced.status.success(), "stderr: {}", stderr(&replaced));
    assert!(
        ws.join("decks/geo.md.bak").exists(),
        "fixture: a bak exists"
    );

    let out = alix(&["deck", "remove", deck.to_str().unwrap(), "--yes"]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("cannot be undone") || stdout(&out).contains("nothing was backed up"),
        "the removal names its finality: {}",
        stdout(&out)
    );
    assert!(!deck.exists(), "deck gone");
    assert!(!ws.join("decks/geo.md.bak").exists(), "backup gone too");
}

#[test]
fn deck_restore_round_trips_a_forced_import() {
    let dir = TempDir::new().unwrap();
    let (ws, deck) = imported_workspace_deck(dir.path());
    let original = std::fs::read_to_string(&deck).unwrap();
    let tsv2 = write(dir.path(), "cards2.tsv", "Q2\tA2\n");
    let replaced = alix(&[
        "deck",
        "import",
        &tsv2,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "geo",
        "--force",
    ]);
    assert!(replaced.status.success(), "stderr: {}", stderr(&replaced));
    let replacement = std::fs::read_to_string(&deck).unwrap();
    assert_ne!(
        original, replacement,
        "fixture: the replace changed the deck"
    );

    let restored = alix(&["deck", "restore", deck.to_str().unwrap()]);
    assert!(restored.status.success(), "stderr: {}", stderr(&restored));
    assert_eq!(
        original,
        std::fs::read_to_string(&deck).unwrap(),
        "the original deck text is live again"
    );
    assert_eq!(
        replacement,
        std::fs::read_to_string(ws.join("decks/geo.md.bak")).unwrap(),
        "the replacement is preserved as the new backup"
    );

    let again = alix(&["deck", "restore", deck.to_str().unwrap()]);
    assert!(again.status.success(), "stderr: {}", stderr(&again));
    assert_eq!(
        replacement,
        std::fs::read_to_string(&deck).unwrap(),
        "restore is its own inverse"
    );
}

#[test]
fn doctor_reports_backup_files_and_the_flag_deletes_them_after_confirmation() {
    let dir = TempDir::new().unwrap();
    let (ws, _deck) = imported_workspace_deck(dir.path());
    let tsv2 = write(dir.path(), "cards2.tsv", "Q2\tA2\n");
    let replaced = alix(&[
        "deck",
        "import",
        &tsv2,
        "--workspace",
        ws.to_str().unwrap(),
        "--output",
        "geo",
        "--force",
    ]);
    assert!(replaced.status.success(), "stderr: {}", stderr(&replaced));
    let bak = ws.join("decks/geo.md.bak");
    assert!(bak.exists(), "fixture: a backup exists");

    // The report: a warning-grade finding naming both remedies, exit still 0.
    let report = alix(&["doctor", ws.to_str().unwrap()]);
    assert!(
        report.status.success(),
        "backups warn, never fail: stderr: {}",
        stderr(&report)
    );
    let out = stdout(&report);
    assert!(out.contains("backup file(s)"), "stdout: {out}");
    assert!(out.contains("deck restore"), "stdout: {out}");
    assert!(out.contains("--remove-backup-files"), "stdout: {out}");

    // The flag without --yes refuses headless and deletes nothing.
    let refused = alix(&["doctor", ws.to_str().unwrap(), "--remove-backup-files"]);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("refusing without a terminal"),
        "stderr: {}",
        stderr(&refused)
    );
    assert!(bak.exists(), "nothing deleted without confirmation");

    // With --yes it deletes and says so; a rerun reports a clean tree.
    let cleaned = alix(&[
        "doctor",
        ws.to_str().unwrap(),
        "--remove-backup-files",
        "--yes",
    ]);
    assert!(cleaned.status.success(), "stderr: {}", stderr(&cleaned));
    assert!(
        stdout(&cleaned).contains("Deleted 1 backup file(s)."),
        "stdout: {}",
        stdout(&cleaned)
    );
    assert!(!bak.exists(), "the backup is gone");
    let again = alix(&[
        "doctor",
        ws.to_str().unwrap(),
        "--remove-backup-files",
        "--yes",
    ]);
    assert!(
        stdout(&again).contains("No backup files"),
        "stdout: {}",
        stdout(&again)
    );
}

#[test]
fn deck_remove_then_restore_is_a_clean_error() {
    let dir = TempDir::new().unwrap();
    let (_ws, deck) = imported_workspace_deck(dir.path());
    let removed = alix(&["deck", "remove", deck.to_str().unwrap(), "--yes"]);
    assert!(removed.status.success(), "stderr: {}", stderr(&removed));

    let out = alix(&["deck", "restore", deck.to_str().unwrap()]);

    assert!(
        !out.status.success(),
        "a removed deck has no backups to swap"
    );
    assert!(
        stderr(&out).contains("geo.md.bak"),
        "the error names the missing backup: {}",
        stderr(&out)
    );
}

// ── `alix workspace init` ────────────────────────────────────────────────────

#[test]
fn workspace_init_on_an_existing_workspace_errors() {
    let dir = TempDir::new().unwrap();
    let ws = dir.path().join("fresh");
    let first = alix(&["workspace", "init", ws.to_str().unwrap()]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    let deck = write(&ws, "a.md", "## q\na\n");
    let initialized = alix(&["deck", "init", &deck]);
    assert!(
        initialized.status.success(),
        "stderr: {}",
        stderr(&initialized)
    );
    let second = alix(&["workspace", "init", ws.to_str().unwrap()]);
    assert!(!second.status.success());
    assert!(
        stderr(&second).contains("is already a workspace"),
        "stderr: {}",
        stderr(&second)
    );
}

#[test]
fn workspace_update_previews_then_applies_without_a_second_backend_call() {
    let dir = TempDir::new().unwrap();
    // Staging canonicalizes the workspace root; on macOS the raw tempdir path
    // traverses /var -> /private/var, so the embedded `source:` must be the
    // canonical form or the preserved-source comparison spuriously differs.
    let root = dir.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    let source = root.join("source");
    std::fs::create_dir_all(workspace.join("decks")).unwrap();
    std::fs::create_dir(&source).unwrap();
    std::fs::write(workspace.join("alix.toml"), "").unwrap();
    std::fs::write(source.join("facts.rs"), "old\nnew\n").unwrap();
    let deck = workspace.join("decks/facts.md");
    std::fs::write(
        &deck,
        format!(
            "---\nformat-version: 1\nid: \"deck-deck1\"\nsource: {}\n---\n## Old? <!-- id: card-oldcard -->\nold\n<!-- at: facts.rs:1 -->\n",
            alix::parser::yaml_quote(source.to_str().unwrap())
        ),
    )
    .unwrap();
    alix::assets::freeze_member(&deck).unwrap();
    let proposal = format!(
        "---\nformat-version: 1\nid: \"deck-deck1\"\nsource: {}\n---\n## New?\nnew\n<!-- at: facts.rs:2 -->\n",
        alix::parser::yaml_quote(source.to_str().unwrap())
    );
    let cli = fake_claude(dir.path(), &proposal);
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );

    let preview = alix(&[
        "workspace",
        "update",
        workspace.to_str().unwrap(),
        "--config",
        &config,
    ]);

    assert!(preview.status.success(), "stderr: {}", stderr(&preview));
    assert!(
        stdout(&preview).contains("1 retired, 1 new"),
        "stdout: {}",
        stdout(&preview)
    );
    assert!(
        alix::deck::Deck::load(&deck)
            .unwrap()
            .cards
            .iter()
            .any(|card| card.id().as_deref() == Some("card-oldcard"))
    );
    std::fs::remove_file(&cli).unwrap();

    let applied = alix(&[
        "workspace",
        "update",
        workspace.to_str().unwrap(),
        "--apply",
    ]);

    assert!(applied.status.success(), "stderr: {}", stderr(&applied));
    let updated = alix::deck::Deck::load(&deck).unwrap();
    assert_eq!("New?", updated.cards[0].front);
    assert_ne!(Some("card-oldcard"), updated.cards[0].id().as_deref());
    assert!(!alix::workspace_update::staging_path(&workspace).exists());
}

#[test]
fn workspace_update_discard_leaves_the_workspace_untouched() {
    let dir = TempDir::new().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("alix.toml"), "").unwrap();
    let staging = alix::workspace_update::staging_path(&workspace);
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("proposal"), "candidate").unwrap();

    let out = alix(&[
        "workspace",
        "update",
        workspace.to_str().unwrap(),
        "--discard",
    ]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(!staging.exists());
    assert!(workspace.join("alix.toml").is_file());
}

// ── `alix reset`: remaining branches ─────────────────────────────────────────

#[test]
fn reset_without_target_or_flags_errors() {
    let out = alix(&["reset"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("name a deck, folder, or workspace to reset"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn reset_all_on_an_empty_store_reports_nothing_to_reset() {
    let dir = TempDir::new().unwrap();
    let store = dir.path().join("state");
    let out = alix(&[
        "reset",
        "--all",
        "--yes",
        "--store",
        store.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("No stored progress to reset."),
        "{}",
        stdout(&out)
    );
}

#[test]
fn reset_by_token_card_id_without_a_target() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let store_path = dir.path().join("state");
    let card_id = alix::deck::Deck::load(&deck).unwrap().cards[0]
        .id()
        .unwrap();
    let mut store = deck_store(&deck, &store_path);
    store.get_or_insert(&card_id, 0);
    store.save().unwrap();

    let out = alix(&[
        "reset",
        "--card",
        &card_id,
        "--yes",
        "--store",
        store_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("Reset 1 card(s)."),
        "{}",
        stdout(&out)
    );
}

#[test]
fn reset_by_text_query_within_a_target_resets_only_matching_cards() {
    let dir = TempDir::new().unwrap();
    let deck_text = "---\nformat-version: 1\nid: deck-geography\n---\n## Capital of Japan? <!-- id: card-gj1 -->\nTokyo\n\n## Largest planet? <!-- id: card-gp1 -->\nJupiter\n";
    let deck = write(dir.path(), "geo.md", deck_text);
    let cards = alix::parser::parse_str("geo.md", deck_text).unwrap();
    let store_path = dir.path().join("state");
    let mut store = deck_store(&deck, &store_path);
    for c in &cards {
        store.get_or_insert(&c.id().unwrap(), 0);
    }
    store.save().unwrap();

    let out = alix(&[
        "reset",
        &deck,
        "--card",
        "japan",
        "--yes",
        "--store",
        store_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("Reset 1 card(s)."),
        "{}",
        stdout(&out)
    );

    let reloaded = deck_store(&deck, &store_path);
    assert!(
        reloaded.get(&cards[0].id().unwrap()).is_none(),
        "the matched card should be cleared"
    );
    assert!(
        reloaded.get(&cards[1].id().unwrap()).is_some(),
        "the other card should survive"
    );
}

#[test]
fn reset_by_text_query_with_no_match_reports_nothing() {
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "geo.md",
        "---\nformat-version: 1\nid: deck-geography\n---\n## Capital of Japan? <!-- id: card-gj1 -->\nTokyo\n",
    );
    let store_path = dir.path().join("state");
    let out = alix(&[
        "reset",
        &deck,
        "--card",
        "nonexistent-query",
        "--yes",
        "--store",
        store_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("No stored progress matching"),
        "{}",
        stdout(&out)
    );
}

#[cfg(unix)]
#[test]
fn sigterm_flushes_and_exits_the_server_cleanly() {
    let dir = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "d.md",
        "---\nformat-version: 1\nid: deck-sigdeck\n---\n## q <!-- id: card-sig1 -->\na\n",
    );
    assert!(alix(&["deck", "init", &deck]).status.success());

    let mut child = Command::new(env!("CARGO_BIN_EXE_alix"))
        .arg(dir.path())
        .args(["--port", "0"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path())
        .env("XDG_DATA_HOME", home.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn the alix server");

    // Readiness: the URL line prints only after the socket is bound.
    let stdout = child.stdout.take().expect("stdout was piped");
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if line.contains("http://") {
                let _ = ready_tx.send(());
                break;
            }
        }
    });
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the server never printed its URL");

    assert!(
        Command::new("kill")
            .arg(child.id().to_string())
            .status()
            .expect("failed to run kill")
            .success()
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("failed to poll the server") {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the server did not exit after SIGTERM"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert!(
        status.success(),
        "SIGTERM must drain and exit cleanly, not kill the process: {status:?}"
    );
}

#[cfg(unix)]
#[test]
fn the_http_log_prints_a_timing_line_for_a_served_request() {
    let dir = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "d.md",
        "---\nformat-version: 1\nid: deck-httplog\n---\n## q <!-- id: card-hl1 -->\na\n",
    );
    assert!(alix(&["deck", "init", &deck]).status.success());

    let mut child = Command::new(env!("CARGO_BIN_EXE_alix"))
        .arg(dir.path())
        .args(["--port", "0"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path())
        .env("XDG_DATA_HOME", home.path())
        .env("ALIX_HTTP_LOG", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn the alix server");

    // Readiness: the URL line prints only after the socket is bound.
    let stdout = child.stdout.take().expect("stdout was piped");
    let (url_tx, url_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if line.contains("http://127.0.0.1:") {
                let _ = url_tx.send(line);
                break;
            }
        }
    });
    let url_line = url_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the server never printed its URL");
    let port: u16 = url_line
        .split("http://127.0.0.1:")
        .nth(1)
        .and_then(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .expect("the URL line carries no port");

    let stderr = child.stderr.take().expect("stderr was piped");
    let (log_tx, log_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            if line.contains("[http]") {
                let _ = log_tx.send(line);
                break;
            }
        }
    });

    {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))
            .expect("failed to connect to the served port");
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .expect("failed to send the request");
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response);
    }

    let log_line = log_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("no [http] timing line reached stderr for the request");
    assert!(
        log_line.contains("at=") && log_line.contains("took=") && log_line.contains("GET /"),
        "the timing line must carry at=, took=, and the request line: {log_line}"
    );

    child.kill().expect("failed to stop the server");
    let _ = child.wait();
}

#[test]
fn generate_trace_keeps_a_silent_backends_full_trace_budget() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    write(dir.path(), "notes.md", "some source material\n");
    let reply = dir.path().join("reply.txt");
    std::fs::write(&reply, "## what it is\nsome point\n<!-- at: 1 -->\n").unwrap();
    let cli = dir.path().join("fake-gemini");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\nsleep 2\ncat {}\n",
            reply.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = write(
        dir.path(),
        "config.toml",
        &format!(
            "[ask]\nbackend = \"gemini\"\ncommand = \"{}\"\n\
             [trace]\ntimeout_secs = 20\n\
             [generate]\nidle_timeout_secs = 1\n",
            cli.display()
        ),
    );
    let out_path = dir.path().join("walk.md");

    let out = alix(&[
        "generate",
        dir.path().to_str().unwrap(),
        "--trace",
        "--config",
        &config,
        "--output",
        out_path.to_str().unwrap(),
    ]);

    assert!(
        out.status.success(),
        "a backend alix never puts in a streaming mode must keep its [trace] budget: {}",
        stderr(&out)
    );
}

#[test]
fn workspace_update_stops_a_wedged_provider_at_the_inactivity_limit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    let source = root.join("source");
    std::fs::create_dir_all(workspace.join("decks")).unwrap();
    std::fs::create_dir(&source).unwrap();
    std::fs::write(workspace.join("alix.toml"), "").unwrap();
    std::fs::write(source.join("facts.rs"), "old\nnew\n").unwrap();
    let deck = workspace.join("decks/facts.md");
    std::fs::write(
        &deck,
        format!(
            "---\nformat-version: 1\nid: \"deck-deck1\"\nsource: {}\n---\n## Old? <!-- id: card-oldcard -->\nold\n<!-- at: facts.rs:1 -->\n",
            alix::parser::yaml_quote(source.to_str().unwrap())
        ),
    )
    .unwrap();
    alix::assets::freeze_member(&deck).unwrap();

    let cli = dir.path().join("wedged-claude");
    std::fs::write(
        &cli,
        "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\nexec sleep 600\n",
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = write(
        dir.path(),
        "config.toml",
        &format!(
            "[ask]\ncommand = \"{}\"\n[generate]\ntimeout_secs = 3\nidle_timeout_secs = 1\n",
            cli.display()
        ),
    );

    let out = alix(&[
        "workspace",
        "update",
        workspace.to_str().unwrap(),
        "--config",
        &config,
    ]);

    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("made no progress"),
        "`workspace update` must honour the inactivity limit: {}",
        stderr(&out)
    );
}

#[test]
fn generate_reports_the_providers_own_error_not_the_raw_event_stream() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let stream = dir.path().join("stream.jsonl");
    std::fs::write(
        &stream,
        concat!(
            r#"{"type":"system","subtype":"hook_started","hook_id":"05cd5ae8","hook_name":"SessionStart:startup","hook_event":"SessionStart","uuid":"108cbf45","session_id":"57d5cf05"}"#,
            "\n",
            r#"{"type":"system","subtype":"init","session_id":"57d5cf05","tools":["Read","WebFetch"]}"#,
            "\n",
            r#"{"type":"result","subtype":"success","is_error":true,"result":"There is an issue with the selected model (no-such-model). It may not exist or you may not have access to it."}"#,
            "\n",
        ),
    )
    .unwrap();
    let cli = dir.path().join("fake-claude");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\ncat {}\nexit 1\n",
            stream.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{}\"\n", cli.display()),
    );

    let out = alix(&[
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--print",
    ]);

    assert!(!out.status.success(), "the run must fail");
    assert!(
        stderr(&out).contains("issue with the selected model"),
        "the provider's own error must be reported: {}",
        stderr(&out)
    );
    assert!(
        !stderr(&out).contains("hook_started"),
        "the raw event stream must not be the failure detail: {}",
        stderr(&out)
    );
}

#[test]
fn a_zero_inactivity_limit_does_not_break_every_generation() {
    let dir = TempDir::new().unwrap();
    let cli = fake_claude(dir.path(), "## Generated Q\nGenerated A\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\n[generate]\nidle_timeout_secs = 0\n"),
    );

    let out = alix(&[
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--print",
    ]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("## Generated Q"), "{}", stdout(&out));
}

#[test]
fn a_wedged_unstructured_backend_still_stops_at_the_inactivity_limit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let cli = dir.path().join("wedged-gemini");
    std::fs::write(
        &cli,
        "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\nsleep 30\n",
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = write(
        dir.path(),
        "config.toml",
        &format!(
            "[ask]\nbackend = \"gemini\"\ncommand = \"{}\"\n\
             [generate]\ntimeout_secs = 6\nidle_timeout_secs = 1\n",
            cli.display()
        ),
    );
    let out_path = dir.path().join("deck.md");

    let started = std::time::Instant::now();
    let out = alix(&[
        "generate",
        "https://example.org/page",
        "--config",
        &config,
        "--output",
        out_path.to_str().unwrap(),
    ]);
    let elapsed = started.elapsed();

    assert!(!out.status.success(), "the wedged run somehow succeeded");
    assert!(
        elapsed < std::time::Duration::from_secs(4),
        "an unstructured backend has no inactivity guard, so a wedged provider \
         runs to the absolute limit: gave up after {:?}, not the 1s inactivity \
         limit; with the shipped default that is 3600s where the parent commit \
         stopped at 300s. stderr: {}",
        elapsed,
        stderr(&out)
    );
}

// A prerequisite that fails must fail before the backend is spawned: a paid
// generation should never be spent on a workspace that does not exist or a
// deck that is already there. The fake backend leaves a marker when it runs,
// so "was the model called" is observable rather than inferred.
#[test]
fn a_failed_prerequisite_never_reaches_the_backend() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let marker = dir.path().join("backend-was-called");
    let cli = dir.path().join("fake-claude");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\ntouch {}\necho '## q'\necho a\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = write(
        dir.path(),
        "config.toml",
        &format!(
            "[ask]\nbackend = \"claude\"\ncommand = \"{}\"\n",
            cli.display()
        ),
    );

    let existing = dir.path().join("taken.md");
    std::fs::write(&existing, "## q <!-- id: card-taken1 -->\na\n").unwrap();

    for (shape, extra) in [
        (
            "a workspace that does not exist",
            vec![
                "--workspace".to_string(),
                dir.path().join("no-such-workspace").display().to_string(),
            ],
        ),
        (
            "a deck that already exists",
            vec!["--output".to_string(), existing.display().to_string()],
        ),
    ] {
        let _ = std::fs::remove_file(&marker);
        let mut argv = vec![
            "generate".to_string(),
            "https://example.org/page".to_string(),
            "--config".to_string(),
            config.clone(),
        ];
        argv.extend(extra);
        let out = alix(&argv.iter().map(String::as_str).collect::<Vec<_>>());

        assert!(
            !out.status.success(),
            "{shape}: the run succeeded instead of refusing: {}",
            stderr(&out)
        );
        assert!(
            !marker.exists(),
            "{shape}: the backend was spawned before the prerequisite was checked, \
             so a paid call was spent on a run that could never succeed. stderr: {}",
            stderr(&out)
        );
    }
}
