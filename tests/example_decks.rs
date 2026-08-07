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

fn doctor_example(relative_path: &str) -> String {
    let deck = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    assert!(
        deck.is_file(),
        "shipped example deck is missing: {}",
        deck.display()
    );
    let run = Command::new(env!("CARGO_BIN_EXE_alix"))
        .args(["doctor"])
        .arg(&deck)
        .output()
        .expect("failed to run the alix binary");
    let report =
        String::from_utf8_lossy(&run.stdout).into_owned() + &String::from_utf8_lossy(&run.stderr);
    assert!(
        run.status.success(),
        "alix doctor failed on {}; the shipped example deck no longer validates\n{report}",
        deck.display()
    );
    report
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
    doctor_example("docs/examples/workspace-showcase/decks/ownership-move.md");
}

/// One deck per row of `docs/card-shapes.md`. Doctor proves each parses;
/// that each still PRODUCES the shape it advertises is asserted through the
/// API in `tests/api.rs`, because a file that parses can still teach the
/// wrong thing.
#[test]
fn every_shape_example_still_checks() {
    for shape in [
        "table",
        "reveal-line",
        "draw",
        "cloze",
        "authored-choices",
        "direction-both",
        "plain",
    ] {
        doctor_example(&format!("docs/examples/shapes/{shape}.md"));
    }
}

/// The showcase deliberately carries one malformed formula, so it can show
/// what a malformed formula renders as. Pinning the warning keeps that card
/// honest in both directions: a silent detector and a "tidied" example both
/// fail here instead of drifting.
#[test]
fn math_rendering_showcase_still_checks_and_still_demonstrates_malformed_math() {
    let report = doctor_example("docs/examples/syntax/math-rendering.md");

    assert!(
        report.contains("malformed LaTeX math in answer `\\frac{1`"),
        "the showcase must still demonstrate malformed math:\n{report}"
    );
    assert!(
        report.contains("1 warning(s)"),
        "exactly the demonstrated warning, nothing new:\n{report}"
    );
}
