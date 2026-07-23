//! Guards that the example decks committed in the repo still pass `alix deck check`.
//!
//! These decks are referenced from the README and the book and are meant to be
//! runnable, so a change that breaks them should fail CI rather than rot
//! silently. Pre-1.0 the deck format is unstable, and this is the tripwire for a
//! format/parse regression reaching a shipped example.
//!
//! Scope note: `alix doctor` validates syntax, duplicate answers, and that each
//! trace `<!-- at: -->` locator *resolves* (the file exists and the line range is in
//! bounds). It does NOT verify the cited lines still show the code the checkpoint
//! describes, so this test cannot catch *semantic* drift of a live-source trace —
//! only format breakage and locators that fall out of the file entirely.

use std::{path::Path, process::Command};

fn doctor_example(relative_path: &str) {
    let deck = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    assert!(
        deck.is_file(),
        "shipped example deck is missing: {}",
        deck.display()
    );
    let status = Command::new(env!("CARGO_BIN_EXE_alix"))
        .args(["doctor"])
        .arg(&deck)
        .status()
        .expect("failed to run the alix binary");
    assert!(
        status.success(),
        "alix doctor failed on {}; the shipped example deck no longer validates",
        deck.display()
    );
}

#[test]
fn workspace_showcase_example_still_checks() {
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/examples/workspace-showcase/alix.toml");
    assert!(
        manifest.is_file(),
        "workspace showcase manifest is missing: {}",
        manifest.display()
    );
    doctor_example("docs/examples/workspace-showcase/ownership-move.md");
}

#[test]
fn math_rendering_showcase_still_checks() {
    doctor_example("docs/examples/math-rendering-showcase.md");
}
