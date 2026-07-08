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
    let out = alix(&["deck", "check", &deck]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("1 cards"), "stdout: {}", stdout(&out));
}

#[test]
fn a_deck_file_argument_errors_with_a_picker_pointer() {
    // `alix <deck>` was removed — the picker is the one way into a review. The
    // guard fires before any server binds, so this is testable headless.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "a.txt", VALID_DECK);
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
    // `alix review x.txt` now parses as a launcher path plus an unexpected
    // extra positional — the subcommand no longer exists.
    let out = alix(&["review", "x.txt"]);
    assert!(!out.status.success(), "the review subcommand should be gone");
}

#[test]
fn check_rejects_a_malformed_deck() {
    let dir = TempDir::new().unwrap();
    // A card front with no answer line is a parse error.
    let deck = write(dir.path(), "broken.txt", "# a front with no answer\n");
    let out = alix(&["deck", "check", &deck]);
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
        r#"{"version":1,"cards":{"123":{"acquired_ms":0}}}"#,
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

/// A minimal virtual card belonging to `parent`, for seeding a store directly
/// via the lib rather than hand-authoring its on-disk JSON shape. Its id is the
/// `Card::id` of the card `parse(parent, text)` yields — identical to a deck card.
fn sample_virtual_card(parent: &str) -> alix::store::VirtualCard {
    let text = "# front\n\tback\n";
    let id = alix::parser::parse_str(parent, text).unwrap()[0].id();
    alix::store::VirtualCard {
        id,
        kind: alix::store::VirtualKind::Remediation,
        parent: parent.to_string(),
        text: text.to_string(),
        created_ms: 0,
    }
}

#[test]
fn reset_all_clears_virtual_cards() {
    // A store holding ONLY virtual cards must still be reset by `--all`: the
    // count sees the virtual card's schedule in `store.cards` (seeded beside the
    // sidecar entry), and the clear must also drop its sidecar content.
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("progress.json");
    let mut store = alix::store::Store::open(&store_path).unwrap();
    let vc = sample_virtual_card("math.txt");
    let id = vc.id;
    store.insert_virtual(vc);
    store.get_or_insert(id, 0); // the virtual card's schedule lives in store.cards
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

    let reloaded = alix::store::Store::open(&store_path).unwrap();
    assert_eq!(0, reloaded.iter_virtual_cards().count());
}

#[test]
fn deck_reset_drops_that_decks_virtual_cards() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.txt", VALID_DECK);
    let store_path = dir.path().join("progress.json");

    let mut store = alix::store::Store::open(&store_path).unwrap();
    let math_vc = sample_virtual_card("math.txt");
    let math_id = math_vc.id;
    store.insert_virtual(math_vc);
    let other_vc = sample_virtual_card("other.txt");
    let other_id = other_vc.id;
    store.insert_virtual(other_vc);
    store.save().unwrap();

    let out = alix(&[
        "reset",
        &deck,
        "--yes",
        "--store",
        store_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let reloaded = alix::store::Store::open(&store_path).unwrap();
    assert!(
        reloaded.get_virtual(math_id).is_none(),
        "the reset deck's own virtual card should be dropped"
    );
    assert!(
        reloaded.get_virtual(other_id).is_some(),
        "another deck's virtual card should survive"
    );
}

#[test]
fn deck_reset_without_yes_leaves_store_unchanged() {
    // A declined/failed confirmation must not partially apply the reset: the
    // deck's mastered flag, its virtual card, and its authored progress must
    // all still be there afterwards, byte-for-byte.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.txt", VALID_DECK);
    let store_path = dir.path().join("progress.json");

    let card_id = alix::deck::Deck::load(&deck).unwrap().cards[0].id();
    let mut store = alix::store::Store::open(&store_path).unwrap();
    store.get_or_insert(card_id, 0);
    store.set_deck_mastered("math.txt", 0);
    store.insert_virtual(sample_virtual_card("math.txt"));
    store.save().unwrap();
    let before = std::fs::read_to_string(&store_path).unwrap();

    // No `--yes` and no TTY in the test subprocess: the command must error.
    let out = alix(&["reset", &deck, "--store", store_path.to_str().unwrap()]);
    assert!(
        !out.status.success(),
        "a no-TTY reset without --yes should error"
    );

    let after = std::fs::read_to_string(&store_path).unwrap();
    assert_eq!(
        before, after,
        "the store on disk must be untouched by a declined/failed reset"
    );
    let reloaded = alix::store::Store::open(&store_path).unwrap();
    assert!(reloaded.deck_mastered("math.txt"), "mastered flag wiped");
    assert!(reloaded.get(card_id).is_some(), "authored progress wiped");
    assert_eq!(
        1,
        reloaded.virtual_cards_for("math.txt").len(),
        "virtual card wiped"
    );
}

#[test]
fn confirmed_virtual_only_deck_reset_clears_virtual() {
    // A deck with ONLY a virtual card (no authored progress, not mastered) must
    // still have that virtual card cleared and persisted on a confirmed reset.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.txt", VALID_DECK);
    let store_path = dir.path().join("progress.json");

    let mut store = alix::store::Store::open(&store_path).unwrap();
    store.insert_virtual(sample_virtual_card("math.txt"));
    store.save().unwrap();

    let out = alix(&[
        "reset",
        &deck,
        "--yes",
        "--store",
        store_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let reloaded = alix::store::Store::open(&store_path).unwrap();
    assert_eq!(0, reloaded.virtual_cards_for("math.txt").len());
}

#[test]
fn a_progress_file_of_any_version_loads_pre_1_0() {
    // Pre-1.0 there is no store version fence: a store written by a "newer" alix
    // (higher version) loads best-effort rather than being refused — we break the
    // format freely and never bump or migrate the version.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.txt", VALID_DECK);
    let store = write(dir.path(), "progress.json", r#"{"version":999,"cards":{}}"#);

    let out = alix(&["stats", &deck, "--store", &store]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
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
fn augment_target_format_also_covers_a_decks_virtual_card() {
    // A deck's synthesized virtual (remediation) cards get the same format
    // treatment as its authored ones — `set_format` keys by the synth card's
    // real `Card::id`, so a re-drilled remediation card is reshaped too.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "parts.txt", "# List the parts\n    A, B, C\n");

    let store_path = dir.path().join("p.json");
    let mut store = alix::store::Store::open(&store_path).unwrap();
    let vc = sample_virtual_card("parts.txt");
    let virtual_id = vc.id;
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

    let cached = std::fs::read_to_string(dir.path().join("augment.json")).unwrap();
    assert!(
        cached.contains(&virtual_id.to_string()),
        "augment.json should key a format entry by the virtual card's id: {cached}"
    );
    assert!(cached.contains("\"X\""), "augment.json: {cached}");
}

#[test]
fn augment_target_format_skips_an_orphaned_virtual_card_colliding_with_a_real_deck_card() {
    // A partial cloze promote can leave an orphaned sidecar `virtual_cards`
    // entry whose id collides with a real deck card (see `promote_virtual`).
    // `deck augment --target format` must filter those out exactly like
    // `build_review`'s injection does, or the same card gets warmed twice.
    let dir = TempDir::new().unwrap();
    let deck_text = "# List the parts\n    A, B, C\n";
    let deck = write(dir.path(), "parts.txt", deck_text);
    let real_id = alix::parser::parse_str("parts.txt", deck_text).unwrap()[0].id();

    let store_path = dir.path().join("p.json");
    let mut store = alix::store::Store::open(&store_path).unwrap();
    store.insert_virtual(alix::store::VirtualCard {
        id: real_id, // collides with the real deck card's id — simulates an orphan
        kind: alix::store::VirtualKind::Remediation,
        parent: "parts.txt".to_string(),
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
    let out = alix(&["deck", "generate", src, "--config", &config, "--print"]);
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
    let out = alix(&[
        "deck", "generate", src, "--yes", "--config", &config, "--print",
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

#[test]
fn deck_check_validates_like_the_old_check() {
    // `alix deck check <deck>` must parse and report cards exactly as the old
    // `alix check <deck>` did.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.txt", VALID_DECK);
    let out = alix(&["deck", "check", &deck]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("1 cards"), "stdout: {}", stdout(&out));
}

#[test]
fn bare_check_is_gone() {
    // After the move, `alix check <deck>` is no longer a valid subcommand.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.txt", VALID_DECK);
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
fn both_depths_due_card(card_id: u64) -> String {
    format!(
        r#""{card_id}":{{"acquired_ms":1000,"recall":{{"stability":10.0,"difficulty":5.0,"reps":5,"lapses":0,"state":2,"scheduled_days":20,"last_review_ms":1000,"due_ms":2000,"learning_goods":2}},"reconstruct":{{"stability":8.0,"difficulty":5.0,"reps":3,"lapses":0,"state":1,"scheduled_days":10,"last_review_ms":1000,"due_ms":2000,"learning_goods":1}},"recognized_ms":1000,"total_reviews":5,"total_passes":5}}"#
    )
}

#[test]
fn list_shows_per_depth_labels_and_recognized_mark() {
    let dir = TempDir::new().unwrap();
    // Card 1: recall=review (state 2), reconstruct=learning (state 1), recognized.
    // Card 2: recall=learning only — no reconstruct schedule, not recognized.
    let deck_text = "# Q1\n\tA1\n\n# Q2\n\tA2\n";
    let deck = write(dir.path(), "cards.txt", deck_text);
    let cards = alix::parser::parse_str("cards.txt", deck_text).unwrap();
    let (id1, id2) = (cards[0].id(), cards[1].id());
    let card1 = both_depths_due_card(id1);
    let card2 = format!(
        r#""{id2}":{{"acquired_ms":1000,"recall":{{"stability":1.0,"difficulty":5.0,"reps":1,"lapses":0,"state":1,"scheduled_days":0,"last_review_ms":1000,"due_ms":2000,"learning_goods":1}},"total_reviews":1,"total_passes":1}}"#
    );
    let store = write(
        dir.path(),
        "store.json",
        &format!(r#"{{"version":1,"cards":{{{card1},{card2}}}}}"#),
    );
    let out = alix(&["list", &deck, "--store", &store]);
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
    let deck_text = "# Q1\n\tA1\n";
    let deck = write(dir.path(), "stats.txt", deck_text);
    let card_id = alix::parser::parse_str("stats.txt", deck_text).unwrap()[0].id();
    let card = both_depths_due_card(card_id);
    let store = write(
        dir.path(),
        "store.json",
        &format!(r#"{{"version":1,"cards":{{{card}}}}}"#),
    );
    let out = alix(&["stats", &deck, "--store", &store]);
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
