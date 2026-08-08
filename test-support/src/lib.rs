//! One fixture and one expectation, shared by the web server's tests and the
//! mobile bridge's tests.
//!
//! The two surfaces cannot be driven from one test binary: the bridge builds
//! the lean core and the server lives behind `full`. So each asserts the same
//! durable effects from here, and a divergence surfaces as one side failing.
//!
//! Every expected value is a literal. Computing one from `alix` would make it
//! move whenever the code under test moves, which is a test that cannot fail.

use std::path::{Path, PathBuf};

pub const DECK_FILE: &str = "parity.md";

/// What the tutor hands back on both surfaces: the server's fake reply is
/// scripted to it, the bridge passes it as an argument.
pub const NOTE: &str = "the condensed insight";

pub const DECK: &str = "---\nformat-version: 1\nid: deck-parityparityparityparit\n---\n\
                        ## what is parity <!-- id: card-parityparityparityparit -->\n\
                        the same effects from either client\n";

/// The durable effects a client action leaves behind. Timestamps and minted
/// tokens are deliberately absent: the two surfaces run in different processes
/// at different times, so only what is stable across both belongs here.
#[derive(Debug, PartialEq, Eq)]
pub struct Effects {
    pub deck: String,
    pub sidecar: Option<String>,
}

pub fn seed(dir: &Path) -> PathBuf {
    let deck = dir.join(DECK_FILE);
    std::fs::write(&deck, DECK).expect("the parity fixture is writable");
    deck
}

pub fn capture(deck: &Path) -> Effects {
    Effects {
        deck: std::fs::read_to_string(deck).expect("the deck survives every action"),
        sidecar: std::fs::read_to_string(deck.with_extension("personal.md")).ok(),
    }
}

/// Taking a tutor note: the authored deck is untouched, and the note opens a
/// block addressed to the card it was taken against.
pub fn after_note() -> Effects {
    Effects {
        deck: DECK.to_string(),
        sidecar: Some(
            "---\nformat-version: 1\nfor: deck-parityparityparityparit\n---\n\n\
             <!-- note: card-parityparityparityparit -->\n> the condensed insight\n"
                .to_string(),
        ),
    }
}
