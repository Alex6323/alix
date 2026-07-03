//! Guards that the example decks committed in the repo still pass `alix check`.
//!
//! These decks are referenced from the README and the book and are meant to be
//! runnable, so a change that breaks them should fail CI rather than rot
//! silently. Pre-1.0 the deck format is unstable, and this is the tripwire for a
//! format/parse regression reaching a shipped example.
//!
//! Scope note: `alix check` validates syntax, duplicate answers, and that each
//! trace `% at:` locator *resolves* (the file exists and the line range is in
//! bounds). It does NOT verify the cited lines still show the code the checkpoint
//! describes, so this test cannot catch *semantic* drift of a live-source trace —
//! only format breakage and locators that fall out of the file entirely.

use std::{path::Path, process::Command};

#[test]
fn keypress_to_grade_example_still_checks() {
    let deck = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/examples/keypress-to-grade.txt");
    let status = Command::new(env!("CARGO_BIN_EXE_alix"))
        .arg("check")
        .arg(&deck)
        .status()
        .expect("failed to run the alix binary");
    assert!(
        status.success(),
        "alix check failed on {} — the shipped example deck no longer validates",
        deck.display()
    );
}
