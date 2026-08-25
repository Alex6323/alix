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
                        ## what is parity\n\
                        the same effects from either client\n\
                        <!-- id: card-parityparityparityparit -->\n";

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
    /// Ids whose `introduced_ms` is present, sorted, minted ones normalized.
    /// The presence of the timestamp is compared, never its value.
    pub introduced: Vec<String>,
}

pub fn seed(dir: &Path) -> PathBuf {
    let deck = dir.join(DECK_FILE);
    std::fs::write(&deck, DECK).expect("the parity fixture is writable");
    deck
}

pub fn capture(deck: &Path, store_root: &Path) -> Effects {
    let sidecar = std::fs::read_to_string(deck.with_extension("personal.md")).ok();
    let minted = sidecar.as_deref().map(minted_tokens).unwrap_or_default();
    Effects {
        deck: std::fs::read_to_string(deck).expect("the deck survives every action"),
        sidecar: sidecar.as_deref().map(normalize),
        scheduled: progress(store_root, &minted).into_iter().map(|p| p.0).collect(),
        reviews: progress(store_root, &minted),
        introduced: introduced(store_root, &minted),
    }
}

/// Progress ids are projected through the SIDECAR's minted tokens: an id the
/// sidecar did not mint stays literal, so progress attached to a different
/// card than the minted one cannot masquerade as a coherent mint.
fn project_id(id: &str, minted: &[String]) -> String {
    if minted.iter().any(|token| token == id) {
        MINTED.to_string()
    } else {
        id.to_string()
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

fn progress(store_root: &Path, minted: &[String]) -> Vec<(String, u64, u64)> {
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
            (
                project_id(id, minted),
                count(state, "total_reviews"),
                count(state, "total_passes"),
            )
        })
        .collect();
    rows.sort();
    rows
}

fn introduced(store_root: &Path, minted: &[String]) -> Vec<String> {
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
    let mut ids: Vec<String> = cards
        .iter()
        .filter(|(_, state)| {
            state
                .get("introduced_ms")
                .and_then(serde_json::Value::as_u64)
                .is_some()
        })
        .map(|(id, _)| project_id(id, minted))
        .collect();
    ids.sort();
    ids
}

/// Taking a tutor note: the authored deck is untouched, the note opens a block
/// addressed to the card it was taken against, and nothing is scheduled by it.
/// Being shown the card records nothing either (ADR 0035: presentation writes
/// nothing), so no store entry exists at all.
pub fn after_note() -> Effects {
    Effects {
        deck: DECK.to_string(),
        sidecar: Some(
            "---\nfor: deck-parityparityparityparit\n---\n\n\
             <!-- note: card-parityparityparityparit -->\n> the condensed insight\n"
                .to_string(),
        ),
        scheduled: Vec::new(),
        reviews: Vec::new(),
        introduced: Vec::new(),
    }
}

/// Minting a card the learner keeps: it lands in the personal file as an
/// ordinary card block, introduced at creation (writing it IS meeting it).
/// The card the learner was viewing leaves no entry: presentation writes
/// nothing (ADR 0035).
pub fn after_mint() -> Effects {
    Effects {
        deck: DECK.to_string(),
        sidecar: Some(format!(
            "---\nfor: deck-parityparityparityparit\n---\n\n\
             ## {MINTED_FRONT}\n{MINTED_BACK}\n<!-- id: {MINTED} -->\n"
        )),
        scheduled: vec![MINTED.to_string()],
        reviews: vec![(MINTED.to_string(), 0, 0)],
        introduced: vec![MINTED.to_string()],
    }
}

/// Passing the card: nothing on disk changes, and the one durable trace is in
/// the progress document. Due times are not compared, since the clients inject
/// different clocks; the counts are what both must agree on. The grade
/// supplies the schedule but never the introduction (ADR 0035).
pub fn after_pass() -> Effects {
    Effects {
        deck: DECK.to_string(),
        sidecar: None,
        scheduled: vec![CARD_ID.to_string()],
        reviews: vec![(CARD_ID.to_string(), 1, 1)],
        introduced: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_rejects_progress_attached_to_a_different_id_than_the_minted_card() {
        const SIDECAR_ID: &str = "card-aaaaaaaaaaaaaaaaaaaaaaaaaa";
        const PROGRESS_ID: &str = "card-bbbbbbbbbbbbbbbbbbbbbbbbbb";

        let root = std::env::temp_dir().join(format!(
            "alix-parity-id-repro-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("progress")).unwrap();
        let deck = seed(&root);
        std::fs::write(
            deck.with_extension("personal.md"),
            format!(
                "---\nformat-version: 1\nfor: {DECK_ID}\n---\n\n\
                 ## {MINTED_FRONT}\n{MINTED_BACK}\n<!-- id: {SIDECAR_ID} -->\n"
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("progress").join(format!("{DECK_ID}.json")),
            serde_json::json!({
                "cards": {
                    PROGRESS_ID: {
                        "introduced_ms": 1,
                        "total_reviews": 0,
                        "total_passes": 0
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let captured = capture(&deck, &root);
        std::fs::remove_dir_all(&root).ok();
        assert_ne!(
            after_mint(),
            captured,
            "the parity oracle must preserve enough identity to reject progress for a different card"
        );
    }
}
