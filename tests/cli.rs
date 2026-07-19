//! End-to-end CLI integration tests: each runs the built `alix` binary as a
//! subprocess against temp decks and a temp progress store, asserting on exit
//! status and output. Unlike `tests/calibrate.rs` these are fully deterministic
//! (no real Claude) so they run in CI on every `make check`.
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

const VALID_DECK: &str = "## What is 2 + 2? <!-- id: math1 -->\n4\n";

#[test]
fn check_accepts_a_valid_deck() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let out = alix(&["doctor", &deck]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("1 cards"), "stdout: {}", stdout(&out));
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
    write(ws, "cards.md", VALID_DECK);
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
    assert!(ws.join("assets").is_dir());
}

#[test]
fn workspace_deadline_shows_sets_and_clears() {
    let dir = TempDir::new().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"Ws\"\n").unwrap();
    std::fs::write(ws.join("cards.md"), "## Q?\nA\n").unwrap();

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
    write(dir.path(), "alpha.md", "## a?\na\n");
    write(dir.path(), "beta.md", "## b?\nb\n");
    let store = dir.path().join("elsewhere.json");
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
    // own progress.json — including the mastered flag.
    let dir = TempDir::new().unwrap();
    let ws = dir.path().join("eng");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"Eng\"\n").unwrap();
    let a = write(&ws, "a.md", "## qa <!-- id: qa1 -->\nans-a\n");
    let b = write(&ws, "b.md", "## qb <!-- id: qb1 -->\nans-b\n");
    let store_path = ws.join("progress.json");
    let mut store = alix::store::Store::open(&store_path).unwrap();
    for deck in [&a, &b] {
        let cards = alix::l1::parse_str(
            Path::new(deck).file_name().unwrap().to_str().unwrap(),
            &std::fs::read_to_string(deck).unwrap(),
        )
        .unwrap();
        store.get_or_insert(&cards[0].id().unwrap(), 0);
    }
    store.set_deck_mastered("a.md", 0);
    store.save().unwrap();

    let out = alix(&["reset", ws.to_str().unwrap(), "--yes"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let reloaded = alix::store::Store::open(&store_path).unwrap();
    assert_eq!(0, reloaded.len(), "member card progress should be gone");
    assert!(
        !reloaded.deck_mastered("a.md"),
        "the mastered flag should be cleared"
    );
}

#[test]
fn stats_reports_a_fresh_deck_against_an_empty_store() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
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
/// `Card::id` of the card `parse(parent, text)` yields, identical to a deck
/// card. The literal `<!-- id: -->` token is derived from `parent`: identity is
/// the token now, so two parents' sample cards must not share one.
fn sample_virtual_card(parent: &str) -> alix::store::VirtualCard {
    let token: String = format!(
        "v{}",
        parent
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase()
    );
    let text = format!("## front <!-- id: {token} -->\nback\n");
    let id = alix::l1::parse_str(parent, &text).unwrap()[0].id().unwrap();
    alix::store::VirtualCard {
        id,
        kind: alix::store::VirtualKind::Remediation,
        parent: parent.to_string(),
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
    let store_path = dir.path().join("progress.json");
    let mut store = alix::store::Store::open(&store_path).unwrap();
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

    let reloaded = alix::store::Store::open(&store_path).unwrap();
    assert_eq!(0, reloaded.iter_virtual_cards().count());
}

#[test]
fn orphans_are_never_auto_pruned_and_reset_orphans_clears_them() {
    // Orphaned progress: a store key matching no live card or deck (a stripped
    // id comment, a hand-deleted deck) is evidence. A normal reset never sweeps
    // it; only the explicit `reset --orphans` does.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK); // card id `math1`
    let store_path = dir.path().join("progress.json");

    let mut store = alix::store::Store::open(&store_path).unwrap();
    store.get_or_insert("math1", 0); // the live card
    store.get_or_insert("orphan1", 0); // an orphaned card key
    store.set_last_depth("ghost.md", alix::depth::Depth::Recall); // an orphaned deck key
    store.save().unwrap();

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
    let after = std::fs::read_to_string(&store_path).unwrap();
    assert!(
        !after.contains("math1"),
        "the live card should be reset: {after}"
    );
    assert!(
        after.contains("orphan1"),
        "the orphan card must survive: {after}"
    );
    assert!(
        after.contains("ghost.md"),
        "the orphan deck key must survive: {after}"
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
    let after = std::fs::read_to_string(&store_path).unwrap();
    assert!(
        !after.contains("orphan1"),
        "orphan card not cleared: {after}"
    );
    assert!(
        !after.contains("ghost.md"),
        "orphan deck key not cleared: {after}"
    );
}

#[test]
fn deck_reset_drops_that_decks_virtual_cards() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let store_path = dir.path().join("progress.json");

    let mut store = alix::store::Store::open(&store_path).unwrap();
    let math_vc = sample_virtual_card("math.md");
    let math_id = math_vc.id.clone();
    store.insert_virtual(math_vc);
    let other_vc = sample_virtual_card("other.md");
    let other_id = other_vc.id.clone();
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
        reloaded.get_virtual(&math_id).is_none(),
        "the reset deck's own virtual card should be dropped"
    );
    assert!(
        reloaded.get_virtual(&other_id).is_some(),
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
    let store_path = dir.path().join("progress.json");

    let card_id = alix::deck::Deck::load(&deck).unwrap().cards[0]
        .id()
        .unwrap();
    let mut store = alix::store::Store::open(&store_path).unwrap();
    store.get_or_insert(&card_id, 0);
    store.set_deck_mastered("math.md", 0);
    store.insert_virtual(sample_virtual_card("math.md"));
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
    assert!(reloaded.deck_mastered("math.md"), "mastered flag wiped");
    assert!(reloaded.get(&card_id).is_some(), "authored progress wiped");
    assert_eq!(
        1,
        reloaded.virtual_cards_for("math.md").len(),
        "virtual card wiped"
    );
}

#[test]
fn confirmed_virtual_only_deck_reset_clears_virtual() {
    // A deck with ONLY a virtual card (no authored progress, not mastered) must
    // still have that virtual card cleared and persisted on a confirmed reset.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let store_path = dir.path().join("progress.json");

    let mut store = alix::store::Store::open(&store_path).unwrap();
    store.insert_virtual(sample_virtual_card("math.md"));
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
    assert_eq!(0, reloaded.virtual_cards_for("math.md").len());
}

#[test]
fn a_progress_file_of_any_version_loads_pre_1_0() {
    // Pre-1.0 there is no store version fence: a store written by a "newer" alix
    // (higher version) loads best-effort rather than being refused — we break the
    // format freely and never bump or migrate the version.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
    let store = write(dir.path(), "progress.json", r#"{"version":999,"cards":{}}"#);

    let out = alix(&["stats", &deck, "--store", &store]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
}

#[test]
fn a_corrupt_progress_file_fails_without_overwriting_it() {
    // A damaged store must not be silently replaced with an empty one — the
    // command fails and the bytes on disk are preserved for recovery.
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "math.md", VALID_DECK);
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
    // writes the result to the sidecar `augment.json` beside the store, never
    // rewriting the card TEXT. Augment open is an enumerated stamp site, so the
    // deck's identity is minted (a frontmatter `id:` is added), but the card's
    // own line and its token are left exactly as they were. The Claude call is
    // faked by a config-wired CLI.
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "parts.md",
        "## List the parts <!-- id: parts1 -->\nA, B, C\n",
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
    // The card's own text and token are untouched (format is display-only). The
    // deck was stamped at augment open (a frontmatter `id:` was minted).
    let deck_after = std::fs::read_to_string(&deck).unwrap();
    assert!(
        deck_after.contains("## List the parts <!-- id: parts1 -->\nA, B, C\n"),
        "card text and token preserved: {deck_after}"
    );
    assert!(
        deck_after.starts_with("---\nid: \""),
        "the deck gained a frontmatter id at augment open: {deck_after}"
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
        "## List the parts <!-- id: parts1 -->\nA, B, C\n",
    );

    let store_path = dir.path().join("p.json");
    let mut store = alix::store::Store::open(&store_path).unwrap();
    let vc = sample_virtual_card("parts.md");
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

    let cached = std::fs::read_to_string(dir.path().join("augment.json")).unwrap();
    assert!(
        cached.contains(virtual_id.as_str()),
        "augment.json should key a format entry by the virtual card's id: {cached}"
    );
    assert!(cached.contains("\"X\""), "augment.json: {cached}");
}

#[test]
fn augment_target_format_skips_an_orphaned_virtual_card_colliding_with_a_real_deck_card() {
    // A partial cloze promote can leave an orphaned sidecar `virtual_cards`
    // entry whose id collides with a real deck card (see `promote_virtual`).
    // `deck augment --target format` must filter those out exactly like
    // `assemble::select`'s injection does, or the same card gets warmed twice.
    let dir = TempDir::new().unwrap();
    let deck_text = "## List the parts <!-- id: parts1 -->\nA, B, C\n";
    let deck = write(dir.path(), "parts.md", deck_text);
    let real_id = alix::l1::parse_str("parts.md", deck_text).unwrap()[0]
        .id()
        .unwrap();

    let store_path = dir.path().join("p.json");
    let mut store = alix::store::Store::open(&store_path).unwrap();
    store.insert_virtual(alix::store::VirtualCard {
        id: real_id.clone(), // collides with the real deck card's id — simulates an orphan
        kind: alix::store::VirtualKind::Remediation,
        parent: "parts.md".to_string(),
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
    let deck_text = "## Q1 <!-- id: q1 -->\nA1\n\n## Q2 <!-- id: q2 -->\nA2\n";
    let deck = write(dir.path(), "cards.md", deck_text);
    let cards = alix::l1::parse_str("cards.md", deck_text).unwrap();
    let (id1, id2) = (cards[0].id().unwrap(), cards[1].id().unwrap());
    let card1 = both_depths_due_card(&id1);
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
    let deck_text = "## Q1 <!-- id: q1 -->\nA1\n";
    let deck = write(dir.path(), "stats.md", deck_text);
    let card_id = alix::l1::parse_str("stats.md", deck_text).unwrap()[0]
        .id()
        .unwrap();
    let card = both_depths_due_card(&card_id);
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
    std::fs::write(decks.path().join("q.md"), "## Q\nA\n").unwrap();

    // A config pointing decks_dir at our temp folder.
    let cfg = decks.path().join("config.toml");
    std::fs::write(
        &cfg,
        format!("decks_dir = \"{}\"\n", decks.path().display()),
    )
    .unwrap();
    let cfg = cfg.to_str().unwrap();
    let dir = decks.path().to_str().unwrap();
    let in_folder = decks.path().join("progress.json");
    let in_folder = in_folder.display().to_string();

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

// ── `stats`/`list`/`reset` agree with the served root's store ───────────────

#[test]
fn stats_on_a_loose_deck_resolves_the_decks_dir_root_store_like_review_does() {
    // Bare `alix` (and its picker) reads/writes a loose deck's progress at
    // `<decks_dir>/progress.json`. `alix stats` must resolve the SAME file —
    // proven with a corrupt-file discriminator: if `stats` instead fell back
    // to the global platform store (the pre-fix bug), the folder's corrupt
    // file would never be opened and the command would succeed instead of
    // failing.
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
    std::fs::write(decks.path().join("progress.json"), garbage).unwrap();

    let out = alix(&["stats", &deck, "--config", cfg]);
    assert!(
        !out.status.success(),
        "stats must read <decks_dir>/progress.json, not fall back to the \
         global store: stdout:\n{}",
        stdout(&out)
    );
}

#[test]
fn reset_all_clears_the_decks_dir_root_store_not_the_global_one() {
    // Same discriminator as above, against `reset --all`: it must resolve to
    // `<decks_dir>/progress.json` (mirroring bare `alix`), not the global
    // platform store.
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
    std::fs::write(decks.path().join("progress.json"), garbage).unwrap();

    let out = alix(&["reset", "--all", "--yes", "--config", cfg]);
    assert!(
        !out.status.success(),
        "reset --all must clear <decks_dir>/progress.json, not the global \
         store: stdout:\n{}",
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
fn share_zip_of_a_workspace_folder_strips_personal_state() {
    let dir = TempDir::new().unwrap();
    let ws = dir.path().join("eng");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("alix.toml"), "title = \"Eng\"\n").unwrap();
    write(&ws, "a.md", "## q\na\n");
    write(&ws, "progress.json", "{}"); // must never travel
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
    assert!(landed.join("eng/a.md").is_file());
    assert!(!landed.join("eng/progress.json").exists());
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
    write(&ws, "progress.json", "{}"); // simulates a leak — real `share` never zips this
    let zip_path = src.path().join("eng.zip");
    alix::share::zip_to(&ws, &zip_path).unwrap();

    let out = alix_env(&["receive", zip_path.to_str().unwrap()], home.path(), &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("stripped a leaked personal file: progress.json"),
        "{}",
        stdout(&out)
    );
    assert!(home.path().join("decks/eng/a.md").is_file());
    assert!(!home.path().join("decks/eng/progress.json").exists());
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
    let stub = write(
        dir.path(),
        "t.md",
        "---\ntrace: how it works\nsource: .\n---\n",
    );
    let cli = fake_claude(
        dir.path(),
        "## checkpoint one\nsome point\n<!-- at: 1 -->\n",
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
        "---\ntrace: how it works\nsource: .\n---\n## old checkpoint <!-- id: c1 -->\nold point\n<!-- at: 1 -->\n",
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
    std::fs::create_dir(&ws).unwrap();
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
    assert!(ws.join("gen.md").is_file());
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
        stderr(&out).contains("cards — not written; --print"),
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
    std::fs::create_dir(&ws).unwrap();
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
fn generate_on_a_directory_source_explores_then_falls_back_to_a_single_deck() {
    // A one-item (unparseable-as-a-plan) exploration result routes to a single
    // deck rather than a multi-item workspace build.
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir(&src).unwrap();
    write(&src, "notes.md", "some source material\n");
    let cli = fake_claude(dir.path(), "## Generated Q\nGenerated A\n");
    let config = write(
        dir.path(),
        "config.toml",
        &format!("[ask]\ncommand = \"{cli}\"\ntimeout_secs = 10\n"),
    );
    let ws = dir.path().join("ws");
    std::fs::create_dir(&ws).unwrap();
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
    assert!(ws.join("gen.md").is_file());
}

// ── `alix deck augment`: each target, fake backend ──────────────────────────

#[test]
fn augment_choices_caches_distractors_for_two_cards() {
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "quiz.md",
        "## Q1 <!-- id: q1 -->\nA1\n\n## Q2 <!-- id: q2 -->\nA2\n",
    );
    let cli = fake_claude(dir.path(), r#"{"0": ["W1", "W2"], "1": ["W3", "W4"]}"#);
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
    let cached = std::fs::read_to_string(dir.path().join("augment.json")).unwrap();
    assert!(cached.contains("W1"), "{cached}");
    assert!(cached.contains("W3"), "{cached}");
}

#[test]
fn augment_notes_caches_a_trivia_note() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "quiz.md", "## Q1 <!-- id: q1 -->\nA1\n");
    let cli = fake_claude(dir.path(), r#"{"0": "a fun fact"}"#);
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
        "notes",
        "--store",
        store.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let cached = std::fs::read_to_string(dir.path().join("augment.json")).unwrap();
    assert!(cached.contains("a fun fact"), "{cached}");
}

#[test]
fn augment_without_a_store_flag_caches_beside_the_decks_dir_root_store() {
    // Mirrors `stats_on_a_loose_deck_resolves_the_decks_dir_root_store_like_review_does`:
    // with no `--store`, the cache must land at `<decks_dir>/augment.json` —
    // beside the `<decks_dir>/progress.json` review reads — not the global
    // platform store's sidecar (the pre-fix bug: `common::store_for` had no
    // decks-dir fallback, so a loose deck's augmentations went missing at
    // review time).
    let decks = tempfile::tempdir().unwrap();
    let deck = write(decks.path(), "quiz.md", "## Q1 <!-- id: q1 -->\nA1\n");
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

    let cached = std::fs::read_to_string(decks.path().join("augment.json")).unwrap();
    assert!(cached.contains("a fun fact"), "{cached}");
}

#[test]
fn augment_questions_caches_a_reworded_variant() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "quiz.md", "## Q1 <!-- id: q1 -->\nA1\n");
    let cli = fake_claude(dir.path(), r#"{"0": ["Rephrased Q1?"]}"#);
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
        "questions",
        "--store",
        store.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let cached = std::fs::read_to_string(dir.path().join("augment.json")).unwrap();
    assert!(cached.contains("Rephrased Q1?"), "{cached}");
}

#[test]
fn augment_questions_on_a_cloze_only_deck_errors() {
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "c.md",
        "## Complete <!-- id: c1 -->\nThe capital of France is \\cloze{Paris}.\n",
    );
    let config = write(
        dir.path(),
        "config.toml",
        "[ask]\ncommand = \"/nonexistent/x\"\n",
    );
    let store = dir.path().join("p.json");
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
    let deck = write(dir.path(), "quiz.md", "## Q1 <!-- id: q1 -->\nA1\n");
    let cli = fake_claude(dir.path(), r#"{"0": ["point one", "point two"]}"#);
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
        "keypoints",
        "--store",
        store.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let cached = std::fs::read_to_string(dir.path().join("augment.json")).unwrap();
    assert!(cached.contains("point one"), "{cached}");
    assert!(cached.contains("point two"), "{cached}");
}

#[test]
fn augment_topology_prints_and_caches_the_walk() {
    let dir = TempDir::new().unwrap();
    let deck = write(
        dir.path(),
        "quiz.md",
        "## Q1 <!-- id: q1 -->\nA1\n\n## Q2 <!-- id: q2 -->\nA2\n",
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
    let store = dir.path().join("p.json");
    let out = alix(&[
        "deck",
        "augment",
        &deck,
        "--target",
        "topology",
        "--store",
        store.to_str().unwrap(),
        "--config",
        &config,
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("topology 'pedagogical order'"), "{text}");
    assert!(text.contains("by difficulty"), "{text}");
    assert!(text.contains("(1 topology stored for this deck)"), "{text}");
}

#[test]
fn augment_on_an_empty_deck_errors_without_calling_the_backend() {
    let dir = TempDir::new().unwrap();
    let deck = write(dir.path(), "empty.md", "# Nothing\n");
    let config = write(
        dir.path(),
        "config.toml",
        "[ask]\ncommand = \"/nonexistent/x\"\n",
    );
    let store = dir.path().join("p.json");
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
    assert!(stdout(&out).contains("## Q1"), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("cards — not written; --print"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn deck_import_into_a_workspace_lands_there() {
    let dir = TempDir::new().unwrap();
    let tsv = write(dir.path(), "cards.tsv", "Q1\tA1\n");
    let ws = dir.path().join("ws");
    std::fs::create_dir(&ws).unwrap();
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
    assert!(ws.join("geo.md").is_file());
}

#[test]
fn deck_import_refuses_to_clobber_without_force_then_force_overwrites() {
    let dir = TempDir::new().unwrap();
    let tsv = write(dir.path(), "cards.tsv", "Q1\tA1\n");
    let ws = dir.path().join("ws");
    std::fs::create_dir(&ws).unwrap();
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
    let placed = std::fs::read_to_string(ws.join("geo.md")).unwrap();

    let second = alix(&args);
    assert!(!second.status.success());
    assert!(
        stderr(&second).contains("already exists; pass --force to overwrite"),
        "stderr: {}",
        stderr(&second)
    );
    assert_eq!(
        placed,
        std::fs::read_to_string(ws.join("geo.md")).unwrap(),
        "the deck must be untouched when --force is absent"
    );
    assert!(!ws.join("geo.md.bak").exists());

    // The kept `.md.bak` proves the replace protocol ran; a plain overwrite
    // leaves none.
    let mut forced = args.to_vec();
    forced.push("--force");
    let third = alix(&forced);
    assert!(third.status.success(), "stderr: {}", stderr(&third));
    assert_eq!(
        placed,
        std::fs::read_to_string(ws.join("geo.md.bak")).unwrap()
    );
}

// ── `alix workspace init` ────────────────────────────────────────────────────

#[test]
fn workspace_init_on_an_existing_workspace_errors() {
    let dir = TempDir::new().unwrap();
    let ws = dir.path().join("fresh");
    let first = alix(&["workspace", "init", ws.to_str().unwrap()]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    // `is_workspace` requires a manifest AND at least one deck — a bare `init`
    // alone isn't "already a workspace" yet.
    write(&ws, "a.md", "## q\na\n");
    let second = alix(&["workspace", "init", ws.to_str().unwrap()]);
    assert!(!second.status.success());
    assert!(
        stderr(&second).contains("is already a workspace"),
        "stderr: {}",
        stderr(&second)
    );
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
    let store = dir.path().join("progress.json"); // never created
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
    let store_path = dir.path().join("progress.json");
    let card_id = alix::deck::Deck::load(&deck).unwrap().cards[0]
        .id()
        .unwrap();
    let mut store = alix::store::Store::open(&store_path).unwrap();
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
    let deck_text = "## Capital of Japan? <!-- id: gj1 -->\nTokyo\n\n## Largest planet? <!-- id: gp1 -->\nJupiter\n";
    let deck = write(dir.path(), "geo.md", deck_text);
    let cards = alix::l1::parse_str("geo.md", deck_text).unwrap();
    let store_path = dir.path().join("progress.json");
    let mut store = alix::store::Store::open(&store_path).unwrap();
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

    let reloaded = alix::store::Store::open(&store_path).unwrap();
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
    let deck = write(dir.path(), "geo.md", "## Capital of Japan?\nTokyo\n");
    let store_path = dir.path().join("progress.json");
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
