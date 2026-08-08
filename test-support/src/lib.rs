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
pub const DECK_ID: &str = "deck-parityparityparityparit";
pub const CARD_ID: &str = "card-parityparityparityparit";

/// What the tutor hands back on both surfaces: the server's fake reply is
/// scripted to it, the bridge passes it as an argument.
pub const NOTE: &str = "the condensed insight";

/// A card the learner keeps, minted by whichever client they were using.
pub const MINTED_FRONT: &str = "what did the tutor add";
pub const MINTED_BACK: &str = "a card of the learner's own";

/// Stands in for any minted token. The clients run in different processes, so
/// the tokens themselves differ by design and only their placement is stable.
pub const MINTED: &str = "card-<minted>";

pub const DECK: &str = "---\nformat-version: 1\nid: deck-parityparityparityparit\n---\n\
                        ## what is parity <!-- id: card-parityparityparityparit -->\n\
                        the same effects from either client\n";

/// The durable effects a client action leaves behind. Timestamps are
/// deliberately absent, and minted tokens are normalized: the two surfaces run
/// in different processes at different times, so only what is stable across
/// both belongs here.
#[derive(Debug, PartialEq, Eq)]
pub struct Effects {
    pub deck: String,
    pub sidecar: Option<String>,
    /// Card ids carrying any progress at all, sorted, minted ones normalized.
    pub scheduled: Vec<String>,
    /// Per card, how much review it records: (id, total_reviews, total_passes).
    /// Counts, not due times, because the clients inject different clocks.
    pub reviews: Vec<(String, u64, u64)>,
}

pub fn seed(dir: &Path) -> PathBuf {
    let deck = dir.join(DECK_FILE);
    std::fs::write(&deck, DECK).expect("the parity fixture is writable");
    deck
}

pub fn capture(deck: &Path, store_root: &Path) -> Effects {
    Effects {
        deck: std::fs::read_to_string(deck).expect("the deck survives every action"),
        sidecar: std::fs::read_to_string(deck.with_extension("personal.md"))
            .ok()
            .map(|text| normalize(&text)),
        scheduled: progress(store_root).into_iter().map(|p| p.0).collect(),
        reviews: progress(store_root),
    }
}

/// A minted token is 26 characters after its `card-` prefix, so anything of
/// that shape which is not the seeded card was minted by the run.
fn normalize(text: &str) -> String {
    let mut out = text.to_string();
    for token in minted_tokens(text) {
        out = out.replace(&token, MINTED);
    }
    out
}

fn minted_tokens(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (index, _) in text.match_indices("card-") {
        let token: String = text[index..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if token != CARD_ID && !found.contains(&token) {
            found.push(token);
        }
    }
    found
}

fn progress(store_root: &Path) -> Vec<(String, u64, u64)> {
    let progress = store_root.join("progress").join(format!("{DECK_ID}.json"));
    let Ok(text) = std::fs::read_to_string(&progress) else {
        return Vec::new();
    };
    let Ok(document) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(cards) = document.get("cards").and_then(|c| c.as_object()) else {
        return Vec::new();
    };
    let count = |state: &serde_json::Value, key: &str| {
        state.get(key).and_then(serde_json::Value::as_u64).unwrap_or(0)
    };
    let mut rows: Vec<(String, u64, u64)> = cards
        .iter()
        .map(|(id, state)| {
            let id = if id == CARD_ID { id.clone() } else { MINTED.to_string() };
            (id, count(state, "total_reviews"), count(state, "total_passes"))
        })
        .collect();
    rows.sort();
    rows
}

/// Taking a tutor note: the authored deck is untouched, the note opens a block
/// addressed to the card it was taken against, and nothing is scheduled by it.
pub fn after_note() -> Effects {
    Effects {
        deck: DECK.to_string(),
        sidecar: Some(
            "---\nformat-version: 1\nfor: deck-parityparityparityparit\n---\n\n\
             <!-- note: card-parityparityparityparit -->\n> the condensed insight\n"
                .to_string(),
        ),
        scheduled: vec![CARD_ID.to_string()],
        reviews: vec![(CARD_ID.to_string(), 0, 0)],
    }
}

/// Minting a card the learner keeps: it lands in the personal file as an
/// ordinary card block and carries a schedule of its own.
pub fn after_mint() -> Effects {
    Effects {
        deck: DECK.to_string(),
        sidecar: Some(format!(
            "---\nformat-version: 1\nfor: deck-parityparityparityparit\n---\n\n\
             ## {MINTED_FRONT} <!-- id: {MINTED} -->\n{MINTED_BACK}\n"
        )),
        scheduled: vec![MINTED.to_string(), CARD_ID.to_string()],
        reviews: vec![(MINTED.to_string(), 0, 0), (CARD_ID.to_string(), 0, 0)],
    }
}

/// Passing the card: nothing on disk changes, and the one durable trace is in
/// the progress document. Due times are not compared, since the clients inject
/// different clocks; the counts are what both must agree on.
pub fn after_pass() -> Effects {
    Effects {
        deck: DECK.to_string(),
        sidecar: None,
        scheduled: vec![CARD_ID.to_string()],
        reviews: vec![(CARD_ID.to_string(), 1, 1)],
    }
}
