//! Read-only duplicate identity-token detection across a folder of decks: it
//! never writes; resolution (re-minting the loser's token) happens later at
//! session-open.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::parser;

/// A card token claimed by more than one heading; the keeper keeps its
/// progress, each loser is re-minted at its deck's next review-open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardDupe {
    pub token: String,
    /// The keeper: (deck file, 1-based front line).
    pub keeper: (PathBuf, usize),
    /// The losing cards: (deck file, 1-based front line) each.
    pub losers: Vec<(PathBuf, usize)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DuplicateMap {
    /// (kept deck, excluded deck, shared token) per excluded copy. Never
    /// auto-fixed: the tool can't know which copy is pristine, so removing the
    /// copy's `id:` line is manual.
    pub excluded_decks: Vec<(PathBuf, PathBuf, String)>,
    /// Excludes cards from an already-excluded deck: a whole-file copy is one
    /// deck-level finding, not one per card.
    pub card_dupes: Vec<CardDupe>,
}

pub fn scan_dir(dir: &Path) -> DuplicateMap {
    scan(&crate::workspace::deck_files(dir))
}

/// scan_dir on a token-extracting line scan instead of full parses: the
/// review-open hot path. Divergence from the parser is biased to missing a
/// dup (no resolution, doctor still warns), never to inventing one.
pub fn scan_dir_fast(dir: &Path) -> DuplicateMap {
    let member_dir = crate::workspace::member_dir(dir);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(member_dir)
        .map(|entries| {
            entries
                .filter_map(|r| r.ok().map(|e| e.path()))
                .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "md"))
                .filter(|p| {
                    !p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                        crate::workspace::is_conventional_non_deck(n)
                            || crate::workspace::is_conflict_name(n)
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    let mut parsed = Vec::new();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (deck_token, cards) = extract_ids(&text);
        parsed.push(Parsed {
            path,
            deck_token,
            cards,
        });
    }
    build_map(&parsed)
}

/// (deck token, per-card (token, 1-based heading line)) via a fence-aware
/// line scan mirroring the parser's directive placement rules.
fn extract_ids(text: &str) -> (Option<String>, Vec<(String, usize)>) {
    let mut deck_token = None;
    let mut cards = Vec::new();
    let mut fence: Option<char> = None;
    let mut in_frontmatter = false;
    let mut heading_line = 0usize;
    let mut prev_pipe = false;
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if i == 0 && line.trim_end() == "---" {
            in_frontmatter = true;
            continue;
        }
        if in_frontmatter {
            if line.trim_end().trim_end_matches([' ', '\t']) == "---" {
                in_frontmatter = false;
            } else if deck_token.is_none()
                && let Some(rest) = line.trim().strip_prefix("id:")
            {
                let v = rest.trim().trim_matches('"');
                if matches!(
                    crate::token::parse_id(v),
                    Some((crate::token::Kind::Deck, ..))
                ) {
                    deck_token = Some(v.to_string());
                }
            }
            continue;
        }
        let marker = line.chars().next();
        if (line.starts_with("```") || line.starts_with("~~~")) && fence.is_none() {
            fence = marker;
            continue;
        }
        if let Some(f) = fence {
            if line.starts_with(f) && line.trim_end().chars().all(|c| c == f) {
                fence = None;
            }
            continue;
        }
        if line.starts_with("## ") {
            heading_line = n;
        }
        // A table opens a block too: its first pipe line anchors the
        // container id comment that follows the table.
        if line.starts_with('|') && !prev_pipe {
            heading_line = n;
        }
        prev_pipe = line.starts_with('|');
        let candidate = if line.trim().starts_with("<!--") {
            line.trim()
        } else if line.starts_with("## ")
            && let Some(pos) = line.find("<!--")
        {
            line[pos..].trim()
        } else {
            continue;
        };
        if let Some(inner) = candidate
            .strip_prefix("<!--")
            .and_then(|s| s.strip_suffix("-->"))
            && let Some(rest) = inner.trim().strip_prefix("id:")
        {
            let v = rest.trim();
            if matches!(
                crate::token::parse_prefixed_card_id(v),
                Some((_, None, None, false))
            ) && heading_line > 0
            {
                let entry = (v.to_string(), heading_line);
                if !cards.contains(&entry) {
                    cards.push(entry);
                }
            }
        }
    }
    (deck_token, cards)
}

struct Parsed {
    path: PathBuf,
    deck_token: Option<String>,
    /// One entry per `## ` heading, even though a cloze card's holes (or a
    /// reversed twin) share it.
    cards: Vec<(String, usize)>,
}

/// Skips unreadable/unparseable decks silently; `doctor` reports those
/// separately.
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
        let Ok(deck) = parser::parse(subject, &text) else {
            continue;
        };
        let mut cards: Vec<(String, usize)> = Vec::new();
        for card in &deck.cards {
            // The composed id, not the bare token: sibling table rows share
            // their container token by design and are never duplicates.
            if let Some(id) = card.id() {
                let entry = (id, card.line);
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

    build_map(&parsed)
}

fn build_map(parsed: &[Parsed]) -> DuplicateMap {
    let (excluded_decks, excluded) = deck_dupes(parsed);
    let card_dupes = card_dupes(parsed, &excluded);
    DuplicateMap {
        excluded_decks,
        card_dupes,
    }
}

/// The deck-token duplicates, plus the indices of the losing (excluded) decks.
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

/// The index of the keeper, by [`beats`]: an undecorated stem beats a
/// decorated copy, else the earliest in scan order.
fn keeper_index(paths: &[&Path]) -> usize {
    let mut best = 0;
    for i in 1..paths.len() {
        if beats(paths[i], paths[best]) {
            best = i;
        }
    }
    best
}

/// True only if `current`'s stem is `challenger`'s stem plus a suffix starting
/// non-alphanumeric; an alphanumeric continuation (`deck1` vs `deck10`) is
/// unrelated, not a decoration.
fn beats(challenger: &Path, current: &Path) -> bool {
    let c = stem(challenger);
    let cur = stem(current);
    // `starts_with` guarantees `c.len()` is a char boundary of `cur`, and
    // `c != cur` guarantees a next character exists.
    c != cur
        && cur.starts_with(c.as_str())
        && cur[c.len()..]
            .chars()
            .next()
            .is_some_and(|ch| !ch.is_alphanumeric())
}

/// The file name without its `.md` extension: the unit `beats` compares.
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
            format!(
                "---\nformat-version: 1\nid: \"{token}\"\n---\n## q <!-- id: {card_token} -->\na\n"
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn the_fast_scan_matches_the_full_scan_across_placements_and_fences() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.md"),
            "---\nformat-version: 1\nid: \"deck-dtok1\"\n\n---\n# T\n\n## q1\nanswer\n<!-- id: card-shared1 -->\n\n## q2 <!-- id: card-samelinetok -->\nanswer\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.md"),
            "---\nformat-version: 1\nid: \"deck-dtok2\"\n---\n## q3\n<!-- id: card-shared1 -->\nbelow front\n\n## q4\n```\n## fenced <!-- id: card-fencedtok -->\n<!-- id: card-alsofenced -->\n```\n<!-- id: card-realtok -->\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.md"), "just prose, no cards\n").unwrap();
        std::fs::write(
            dir.path().join("c.md.bak"),
            "## x\n<!-- id: card-shared1 -->\n",
        )
        .unwrap();

        let full = scan_dir(dir.path());
        let fast = scan_dir_fast(dir.path());
        assert_eq!(full, fast);
        assert_eq!(1, fast.card_dupes.len());
        assert_eq!("card-shared1", fast.card_dupes[0].token);
    }

    fn table_deck(deck_id: &str, container: &str) -> String {
        format!(
            "---\nformat-version: 1\nid: \"{deck_id}\"\n---\n| word | meaning |\n|---|---|\n| one | eins | <!-- r:4k2x9w -->\n| two | zwei | <!-- r:7m3p5q -->\n<!-- id: {container} -->\n"
        )
    }

    #[test]
    fn sibling_rows_in_one_table_are_not_duplicate_card_tokens() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("table.md"),
            table_deck(
                "deck-9w2c7x4k1m8q3z5t0v6b2n4d8f",
                "card-4jkya9q3m8z0tw5v9y2b4n6d8f",
            ),
        )
        .unwrap();

        let map = scan_dir(dir.path());

        assert!(map.card_dupes.is_empty(), "{:#?}", map.card_dupes);
    }

    #[test]
    fn the_fast_scan_detects_a_copied_table_container() {
        let dir = tempfile::tempdir().unwrap();
        let container = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        std::fs::write(
            dir.path().join("a.md"),
            table_deck("deck-9w2c7x4k1m8q3z5t0v6b2n4d8f", container),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.md"),
            table_deck("deck-6v3c7x4k1m8q3z5t0b2n4d8f9w", container),
        )
        .unwrap();

        let map = scan_dir_fast(dir.path());

        assert_eq!(1, map.card_dupes.len(), "{map:#?}");
        assert_eq!(container, map.card_dupes[0].token);
        assert_eq!(
            5, map.card_dupes[0].keeper.1,
            "the container anchors to the table's HEADER line, not a later row"
        );
        assert_eq!(vec![5], {
            let mut lines: Vec<usize> = map.card_dupes[0].losers.iter().map(|l| l.1).collect();
            lines.sort_unstable();
            lines
        });
    }

    #[test]
    fn a_duplicate_deck_token_excludes_the_decorated_copy() {
        let dir = tempfile::tempdir().unwrap();
        // Plain lexicographic order would keep `deck (1).md` (space sorts before `.`);
        // the keep-rule must prevent that exact inversion.
        let base = write(dir.path(), "deck.md", "deck-dsame", "card-cbase");
        let copy1 = write(dir.path(), "deck (1).md", "deck-dsame", "card-ccopy1");
        let copy2 = write(dir.path(), "deck copy.md", "deck-dsame", "card-ccopy2");

        let map = scan(&[copy1.clone(), copy2.clone(), base.clone()]);

        assert_eq!(
            vec![
                (base.clone(), copy1, "deck-dsame".to_string()),
                (base, copy2, "deck-dsame".to_string()),
            ],
            map.excluded_decks
        );
        assert!(map.card_dupes.is_empty());
    }

    #[test]
    fn unrelated_duplicate_deck_names_fall_back_to_scan_order() {
        let dir = tempfile::tempdir().unwrap();
        // Non-alphabetical scan order pins that `zebra.md` wins by being scanned
        // first, not by sorting first.
        let zebra = write(dir.path(), "zebra.md", "deck-dsame", "card-czebra");
        let apple = write(dir.path(), "apple.md", "deck-dsame", "card-capple");

        let map = scan(&[zebra.clone(), apple.clone()]);
        assert_eq!(
            vec![(zebra, apple, "deck-dsame".to_string())],
            map.excluded_decks
        );
    }

    #[test]
    fn the_duplicate_map_is_returned_by_the_lib_scan() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("notes.md");
        std::fs::write(
            &a,
            "---\nformat-version: 1\nid: \"deck-dtoka\"\n---\n## q <!-- id: card-cshared -->\na\n",
        )
        .unwrap();
        let b = dir.path().join("notes copy.md");
        std::fs::write(
            &b,
            "---\nformat-version: 1\nid: \"deck-dtokb\"\n---\n## q <!-- id: card-cshared -->\nb\n",
        )
        .unwrap();

        let map = scan(&[a.clone(), b.clone()]);
        assert!(
            map.excluded_decks.is_empty(),
            "decks differ, not deck-dupes"
        );
        assert_eq!(1, map.card_dupes.len());
        let dupe = &map.card_dupes[0];
        assert_eq!("card-cshared", dupe.token);
        assert_eq!((a, 5), dupe.keeper);
        assert_eq!(vec![(b, 5)], dupe.losers);
    }

    #[test]
    fn scan_dir_enumerates_and_skips_unparseable_decks() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "one.md", "deck-d1", "card-c1");
        std::fs::write(dir.path().join("broken.md"), "## q with no answer\n").unwrap();
        let map = scan_dir(dir.path());
        assert!(map.excluded_decks.is_empty());
        assert!(map.card_dupes.is_empty());
    }

    #[test]
    fn two_decorated_copies_without_an_original_fall_back_to_scan_order() {
        let dir = tempfile::tempdir().unwrap();
        // Neither copy's stem is a prefix of the other's, so the keep-rule has
        // nothing to prefer; scan order decides.
        let paren = write(dir.path(), "deck (1).md", "deck-dsame", "card-cparen");
        let copy = write(dir.path(), "deck copy.md", "deck-dsame", "card-ccopy");

        let map = scan(&[paren.clone(), copy.clone()]);
        assert_eq!(
            vec![(paren, copy, "deck-dsame".to_string())],
            map.excluded_decks
        );
    }

    #[test]
    fn case_differing_stems_are_unrelated_decks() {
        let dir = tempfile::tempdir().unwrap();
        // The keep-rule's prefix check is case-sensitive, so differing-case stems
        // count as unrelated names.
        let upper = write(dir.path(), "Deck.md", "deck-dsame", "card-cupper");
        let lower = write(dir.path(), "deck.md", "deck-dsame", "card-clower");

        let map = scan(&[upper.clone(), lower.clone()]);
        assert_eq!(
            vec![(upper, lower, "deck-dsame".to_string())],
            map.excluded_decks
        );
    }

    #[test]
    fn an_alphanumeric_continuation_is_not_a_decoration() {
        let dir = tempfile::tempdir().unwrap();
        let ten = write(dir.path(), "deck10.md", "deck-dsame", "card-cten");
        let one = write(dir.path(), "deck1.md", "deck-dsame", "card-cone");

        let map = scan(&[ten.clone(), one.clone()]);
        assert_eq!(
            vec![(ten, one, "deck-dsame".to_string())],
            map.excluded_decks
        );
    }

    #[test]
    fn fast_scan_ignores_non_decks_by_extension_and_reserved_name() {
        let dir = tempfile::tempdir().unwrap();
        let shared = "card-9w2c7x4k1m8q3z5t0v6b2n4d8f";
        let extension_only = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        let conventional = "card-6v3c7x4k1m8q3z5t0b2n4d8f9w";
        let conflict = "card-8v3c7x4k1m9q2z5t0b6n4d8f9w";
        write(dir.path(), "a.md", "deck-a", shared);
        write(dir.path(), "b.md", "deck-b", shared);
        write(dir.path(), "notes.txt", "deck-c", extension_only);
        write(dir.path(), "extension-peer.md", "deck-d", extension_only);
        write(dir.path(), "README.md", "deck-e", conventional);
        write(dir.path(), "readme-peer.md", "deck-f", conventional);
        write(
            dir.path(),
            "copy.sync-conflict-device.md",
            "deck-g",
            conflict,
        );
        write(dir.path(), "copy-peer.md", "deck-h", conflict);

        let map = scan_dir_fast(dir.path());
        let tokens: Vec<&str> = map
            .card_dupes
            .iter()
            .map(|dupe| dupe.token.as_str())
            .collect();

        assert_eq!(vec![shared], tokens);
    }

    #[test]
    fn fast_id_extraction_obeys_frontmatter_fence_heading_and_token_boundaries() {
        let deck = "deck-9w2c7x4k1m8q3z5t0v6b2n4d8f";
        let before_heading = "card-4jkya9q3m8z0tw5v9y2b4n6d8f";
        let fenced = "card-6v3c7x4k1m8q3z5t0b2n4d8f9w";
        let prose = "card-8v3c7x4k1m9q2z5t0b6n4d8f9w";
        let accepted = "card-7v3c7x4k1m9q2z5t0b6n4d8f9w";
        let text = format!(
            "---\nformat-version: 1\nid: \"{deck}\"\n---\n\
             <!-- id: {before_heading} -->\n\
             ## real\n\
             ```\n\
             ```not a closing fence\n\
             <!-- id: {fenced} -->\n\
             ```\n\
             prose <!-- id: {prose} -->\n\
             <!-- id: card-invalid! -->\n\
             <!-- id: {accepted} -->\n"
        );

        assert_eq!(
            (Some(deck.to_string()), vec![(accepted.to_string(), 6)]),
            extract_ids(&text)
        );
    }
}
