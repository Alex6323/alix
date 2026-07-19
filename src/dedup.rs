//! Duplicate identity-token detection across a folder of decks (spec §2.4).
//!
//! A copied deck file (or a card copied WITH its `<!-- id: -->` comment across
//! decks) makes two `## ` headings claim one token. That is ambiguous: which
//! copy is pristine? This module is the READ-ONLY detector: it never writes.
//! It is a **lib artifact on purpose**: [`scan`] RETURNS the duplicate map, so
//! serve, the CLI, and the frb phone client all see the same result instead of
//! it living only in the web serve loop. Resolution (re-minting the loser's
//! token) is a separate write, done at the session-open site (`assemble`).
//!
//! The keep-rule (which copy wins) is deliberately simple and OWNED: an
//! undecorated/shortest stem beats a decorated superstring of it (`deck.md`
//! beats `deck (1).md` and `deck copy.md`); unrelated names (`notes.md` vs
//! `summary.md` from a plain `cp`) fall back to scan order. The tool cannot
//! know which unrelated copy is pristine, so scan order is honest, not correct.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::l1;

/// A single card token claimed by more than one `## ` heading across the
/// scanned decks (spec §2.4). The keeper keeps the earned progress; each loser
/// is re-minted at its deck's next review-open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardDupe {
    /// The shared identity token (the `<!-- id: -->` value).
    pub token: String,
    /// The card that keeps the token (and its store progress): its deck file
    /// and the 1-based front line.
    pub keeper: (PathBuf, usize),
    /// The colliding cards that lose the token: deck file + 1-based front line.
    pub losers: Vec<(PathBuf, usize)>,
}

/// The duplicate identity tokens found across a folder of decks (spec §2.4),
/// returned by [`scan`] so serve, the CLI, and the frb client all see the same
/// result (never serve-loop-only state). Read-only: producing it writes
/// nothing; resolution happens later at the session-open write site.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DuplicateMap {
    /// A deck token claimed by more than one file: `(kept deck, excluded deck,
    /// the shared deck token)`, one row per excluded copy. The excluded deck is
    /// dropped from enumeration; the fix is deleting the `id:` line in the copy
    /// (never auto-re-minted; the tool cannot know which copy is pristine).
    pub excluded_decks: Vec<(PathBuf, PathBuf, String)>,
    /// Card tokens shared across (non-excluded) decks, each with its keeper and
    /// losers. A deck already excluded above contributes none of its cards here
    /// (a whole-file copy is one deck-level finding, not one per card).
    pub card_dupes: Vec<CardDupe>,
}

/// The `.md` deck files directly in `dir` (the same enumeration the pickers
/// use), scanned for duplicate tokens. The convenience wrapper the
/// session-open and doctor sites call.
pub fn scan_dir(dir: &Path) -> DuplicateMap {
    scan(&crate::workspace::deck_files(dir))
}

/// One parsed deck's identity tokens, read-only.
struct Parsed {
    path: PathBuf,
    deck_token: Option<String>,
    /// Distinct `(card token, 1-based front line)`, in file order. A cloze
    /// card's holes and a reversed twin share one heading, so each `## ` line
    /// contributes at most one entry.
    cards: Vec<(String, usize)>,
}

/// Scan `deck_paths` (in scan order) for duplicate identity tokens (spec §2.4).
/// Read-only: unreadable or unparseable decks are skipped (doctor reports those
/// separately). Deck-token duplicates are found first; a whole-file copy's
/// cards are then left out of the card-token pass, so a copied deck is one
/// deck-level finding rather than a card finding per line.
pub fn scan(deck_paths: &[PathBuf]) -> DuplicateMap {
    let mut parsed: Vec<Parsed> = Vec::new();
    for path in deck_paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let subject = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("deck.md");
        let Ok(deck) = l1::parse_l1(subject, &text) else {
            continue;
        };
        let mut cards: Vec<(String, usize)> = Vec::new();
        for card in &deck.cards {
            if let Some(tok) = card.token.as_deref() {
                let entry = (tok.to_string(), card.line);
                if !cards.contains(&entry) {
                    cards.push(entry);
                }
            }
        }
        parsed.push(Parsed {
            path: path.clone(),
            deck_token: deck.deck_token.clone(),
            cards,
        });
    }

    let (excluded_decks, excluded) = deck_dupes(&parsed);
    let card_dupes = card_dupes(&parsed, &excluded);
    DuplicateMap {
        excluded_decks,
        card_dupes,
    }
}

/// The deck-token duplicates and the set of excluded (loser) deck indices.
fn deck_dupes(parsed: &[Parsed]) -> (Vec<(PathBuf, PathBuf, String)>, Vec<usize>) {
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, p) in parsed.iter().enumerate() {
        if let Some(tok) = p.deck_token.as_deref() {
            groups.entry(tok).or_default().push(i);
        }
    }
    let mut excluded_decks = Vec::new();
    let mut excluded = Vec::new();
    for (tok, idxs) in groups {
        if idxs.len() < 2 {
            continue;
        }
        let keeper = idxs[keeper_index(
            &idxs
                .iter()
                .map(|&i| parsed[i].path.as_path())
                .collect::<Vec<_>>(),
        )];
        for &i in &idxs {
            if i != keeper {
                excluded.push(i);
                excluded_decks.push((
                    parsed[keeper].path.clone(),
                    parsed[i].path.clone(),
                    tok.to_string(),
                ));
            }
        }
    }
    // HashMap iteration is unordered: sort for a deterministic report.
    excluded_decks.sort();
    (excluded_decks, excluded)
}

/// Card-token duplicates across the non-excluded decks.
fn card_dupes(parsed: &[Parsed], excluded: &[usize]) -> Vec<CardDupe> {
    // token -> the sites claiming it, in scan order (deck order, then line).
    let mut sites: HashMap<&str, Vec<(PathBuf, usize)>> = HashMap::new();
    for (i, p) in parsed.iter().enumerate() {
        if excluded.contains(&i) {
            continue;
        }
        for (tok, line) in &p.cards {
            sites
                .entry(tok.as_str())
                .or_default()
                .push((p.path.clone(), *line));
        }
    }
    let mut out = Vec::new();
    for (tok, sites) in sites {
        if sites.len() < 2 {
            continue;
        }
        let keeper = keeper_index(&sites.iter().map(|(p, _)| p.as_path()).collect::<Vec<_>>());
        let mut losers = Vec::new();
        for (i, site) in sites.iter().enumerate() {
            if i != keeper {
                losers.push(site.clone());
            }
        }
        out.push(CardDupe {
            token: tok.to_string(),
            keeper: sites[keeper].clone(),
            losers,
        });
    }
    // Deterministic report order.
    out.sort_by(|a, b| a.token.cmp(&b.token));
    out
}

/// The index of the file that keeps a shared token, by the §2.4 keep-rule:
/// the undecorated/shortest stem wins when one name is a prefix of another (a
/// decorated copy like `deck (1).md` loses to `deck.md`), else the earliest in
/// scan order. `paths` is in scan order.
fn keeper_index(paths: &[&Path]) -> usize {
    let mut best = 0;
    for i in 1..paths.len() {
        if beats(paths[i], paths[best]) {
            best = i;
        }
    }
    best
}

/// Whether `challenger` should replace `current` as the keeper: true only when
/// `current`'s stem is a decorated superstring of `challenger`'s (so the
/// challenger is the undecorated base). Unrelated names and decorated
/// challengers keep `current` (the earlier one, in scan order), which is what
/// makes the fallback order-based, not lexicographic.
fn beats(challenger: &Path, current: &Path) -> bool {
    let c = stem(challenger);
    let cur = stem(current);
    c != cur && cur.starts_with(c.as_str())
}

/// A file name without its `.md` extension (the comparison unit for the
/// keep-rule). The directory is ignored; decks in one folder are the case.
fn stem(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.strip_suffix(".md").unwrap_or(n).to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, token: &str, card_token: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!("---\nid: \"{token}\"\n---\n## q <!-- id: {card_token} -->\na\n"),
        )
        .unwrap();
        path
    }

    #[test]
    fn a_duplicate_deck_token_excludes_the_decorated_copy() {
        let dir = tempfile::tempdir().unwrap();
        // Three files sharing one deck token: the undecorated stem must keep,
        // the `(1)` and `copy` decorations must lose. Plain lexicographic order
        // would keep `deck (1).md` (a space sorts before `.`), the exact
        // inversion the keep-rule prevents.
        let base = write(dir.path(), "deck.md", "dsame", "cbase");
        let copy1 = write(dir.path(), "deck (1).md", "dsame", "ccopy1");
        let copy2 = write(dir.path(), "deck copy.md", "dsame", "ccopy2");

        let map = scan(&[copy1.clone(), copy2.clone(), base.clone()]);

        // Two exclusions, both naming `deck.md` as the keeper.
        assert_eq!(
            vec![
                (base.clone(), copy1, "dsame".to_string()),
                (base, copy2, "dsame".to_string()),
            ],
            map.excluded_decks
        );
        // A whole-file copy is a deck finding, not a card finding per line.
        assert!(map.card_dupes.is_empty());
    }

    #[test]
    fn unrelated_duplicate_deck_names_fall_back_to_scan_order() {
        let dir = tempfile::tempdir().unwrap();
        // `cp zebra.md apple.md`: neither stem is a prefix of the other, so the
        // keeper is the first in SCAN order (owned as order-based), pinned by
        // passing a deliberately non-alphabetical order: `zebra.md` wins
        // because it is scanned first, not because it sorts first.
        let zebra = write(dir.path(), "zebra.md", "dsame", "czebra");
        let apple = write(dir.path(), "apple.md", "dsame", "capple");

        let map = scan(&[zebra.clone(), apple.clone()]);
        assert_eq!(
            vec![(zebra, apple, "dsame".to_string())],
            map.excluded_decks
        );
    }

    #[test]
    fn the_duplicate_map_is_returned_by_the_lib_scan() {
        let dir = tempfile::tempdir().unwrap();
        // Two DISTINCT decks (different deck tokens) that share one CARD token:
        // a card copied WITH its id comment between decks. The lib scan returns
        // it directly — the pin that no dedup state lives only in serve.
        let a = dir.path().join("notes.md");
        std::fs::write(
            &a,
            "---\nid: \"dtoka\"\n---\n## q <!-- id: cshared -->\na\n",
        )
        .unwrap();
        let b = dir.path().join("notes copy.md");
        std::fs::write(
            &b,
            "---\nid: \"dtokb\"\n---\n## q <!-- id: cshared -->\nb\n",
        )
        .unwrap();

        let map = scan(&[a.clone(), b.clone()]);
        assert!(
            map.excluded_decks.is_empty(),
            "decks differ, not deck-dupes"
        );
        assert_eq!(1, map.card_dupes.len());
        let dupe = &map.card_dupes[0];
        assert_eq!("cshared", dupe.token);
        // `notes.md` (undecorated) keeps the token; `notes copy.md` loses it.
        assert_eq!((a, 4), dupe.keeper);
        assert_eq!(vec![(b, 4)], dupe.losers);
    }

    #[test]
    fn scan_dir_enumerates_and_skips_unparseable_decks() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "one.md", "d1", "c1");
        // A broken deck must not panic the scan; it is skipped.
        std::fs::write(dir.path().join("broken.md"), "## q with no answer\n").unwrap();
        let map = scan_dir(dir.path());
        assert!(map.excluded_decks.is_empty());
        assert!(map.card_dupes.is_empty());
    }

    // ---- keep-rule edge pins: what "decorated" requires -------------------
    //
    // `beats` only recognizes decoration as one stem being a literal string
    // prefix of the other (`deck` vs `deck (1)`/`deck copy`). Two stems that
    // are merely unrelated — including two that share a prefix by accident —
    // fall back to scan order, same as `unrelated_duplicate_deck_names_fall_back_to_scan_order`.

    #[test]
    fn two_decorated_copies_without_an_original_fall_back_to_scan_order() {
        let dir = tempfile::tempdir().unwrap();
        // Both copies LOOK decorated (`(1)`, `copy`), but the undecorated
        // `deck.md` original is absent, and neither copy's stem is a prefix of
        // the other's — so the keep-rule has nothing to prefer between them
        // and falls back to scan order (first scanned keeps).
        let paren = write(dir.path(), "deck (1).md", "dsame", "cparen");
        let copy = write(dir.path(), "deck copy.md", "dsame", "ccopy");

        let map = scan(&[paren.clone(), copy.clone()]);
        assert_eq!(vec![(paren, copy, "dsame".to_string())], map.excluded_decks);
    }

    #[test]
    fn case_differing_stems_are_unrelated_decks() {
        let dir = tempfile::tempdir().unwrap();
        // `Deck.md` and `deck.md` differ only by case: the keep-rule's prefix
        // check is byte-for-byte (case-sensitive), so neither is recognized as
        // decorating the other — they are unrelated names, scan order decides.
        let upper = write(dir.path(), "Deck.md", "dsame", "cupper");
        let lower = write(dir.path(), "deck.md", "dsame", "clower");

        let map = scan(&[upper.clone(), lower.clone()]);
        assert_eq!(
            vec![(upper, lower, "dsame".to_string())],
            map.excluded_decks
        );
    }

    // `a_shorter_unrelated_stem_wins_nothing` (deck1 vs deck10, longer scanned
    // first) is INTENTIONALLY NOT PINNED here: it fails against the real
    // `beats()`. `deck10.md` scanned before `deck1.md` yields keeper =
    // `deck1.md`, not `deck10.md` — `beats()`'s decoration check is a bare
    // string-prefix test with no word-boundary/non-alphanumeric guard, so it
    // mistakes `deck10` (an unrelated, independently numbered deck) for a
    // decorated superstring of `deck1`, and lets the shorter stem steal the
    // keeper slot out of scan order. This is a real, verified (executed, not
    // read) edge-case bug in the keep-rule, out of this fix pass's sanctioned
    // scope ("no production change expected"); see
    // `.superpowers/sdd/task-7-report.md` § Fix pass for the repro and BLOCKED
    // status on this one item.
}
