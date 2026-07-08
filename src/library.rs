//! Placing new decks into the library — the shared write tail of `alix
//! generate`, `alix deck import`, and the web equivalents. Validation is
//! lenient on purpose: a generated deck that does not parse yet is still
//! saved (fixable by hand) and the problem is reported, not swallowed.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{deck::Deck, parser, store::Store};

/// What [`place_deck`] wrote: where it landed, how many cards parsed, and the
/// parse problem when the text is not a valid deck yet.
#[derive(Debug)]
pub struct Placed {
    pub path: PathBuf,
    pub cards: usize,
    pub parse_error: Option<String>,
}

/// Writes `text` into `dir` as `<name>.txt` (atomically: `.tmp` + rename).
/// Only `name`'s file-name component is used, so an uploaded name can't
/// traverse; a collision is an error — the caller decides about overwriting.
pub fn place_deck(dir: &Path, name: &str, text: &str) -> Result<Placed> {
    let stem = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("deck");
    let stem = stem.strip_suffix(".txt").unwrap_or(stem);
    let file = format!("{stem}.txt");
    let path = dir.join(&file);
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    // The file name is part of every card's identity hash — parse against it.
    let parsed = parser::parse_str(&file, text);
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let body = if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    };
    let tmp = dir.join(format!(".{file}.tmp"));
    std::fs::write(&tmp, body).with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(match parsed {
        Ok(cards) => Placed {
            path,
            cards: cards.len(),
            parse_error: None,
        },
        Err(e) => Placed {
            path,
            cards: 0,
            parse_error: Some(format!("{e:#}")),
        },
    })
}

/// Wipes all review progress for the given decks from `store` — authored-card
/// schedules, virtual (remediation) cards, and the decks' mastered flags —
/// then saves. Returns how many authored cards had progress. The caller owns
/// any confirmation; nothing here prompts.
pub fn reset_decks<'a>(
    store: &mut Store,
    decks: impl IntoIterator<Item = &'a Deck>,
) -> Result<usize> {
    let mut n = 0;
    for deck in decks {
        store.clear_deck_mastered(&deck.subject);
        let virtual_ids: Vec<u64> = store
            .virtual_cards_for(&deck.subject)
            .iter()
            .map(|vc| vc.id)
            .collect();
        for id in virtual_ids {
            store.remove_virtual(id); // drop sidecar content …
            store.remove(id); // … and the schedule in `store.cards`
        }
        for card in &deck.cards {
            let id = card.id();
            if store.get(id).is_some() {
                store.remove(id);
                n += 1;
            }
        }
    }
    store.save()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placing_a_valid_deck_writes_it_and_counts_cards() {
        let dir = tempfile::tempdir().unwrap();
        let p = place_deck(dir.path(), "rust", "# q\n  a\n").unwrap();
        assert_eq!(dir.path().join("rust.txt"), p.path);
        assert_eq!(1, p.cards);
        assert!(p.parse_error.is_none());
        assert!(p.path.exists());
    }

    #[test]
    fn a_parse_problem_still_writes_the_deck_and_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        // A front with no answer line is invalid but must not be discarded.
        let p = place_deck(dir.path(), "broken.txt", "# q with no answer\n").unwrap();
        assert!(p.path.exists());
        assert!(p.parse_error.is_some());
    }

    #[test]
    fn a_name_collision_errors_without_touching_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("rust.txt"), "original").unwrap();
        let err = place_deck(dir.path(), "rust", "# q\n  a\n").unwrap_err();
        assert!(format!("{err:#}").contains("already exists"), "{err:#}");
        assert_eq!(
            "original",
            std::fs::read_to_string(dir.path().join("rust.txt")).unwrap()
        );
    }

    #[test]
    fn an_uploaded_name_cannot_traverse_out_of_the_dir() {
        let dir = tempfile::tempdir().unwrap();
        let p = place_deck(dir.path(), "../../evil", "# q\n  a\n").unwrap();
        assert!(p.path.starts_with(dir.path()), "{}", p.path.display());
    }

    #[test]
    fn resetting_a_deck_clears_only_that_decks_progress() {
        // Arrange: two decks sharing one store; give each one graded card
        // (mirror the setup helpers in session.rs tests), then:
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "# qa\n\tans-a\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "# qb\n\tans-b\n").unwrap();
        let deck_a = Deck::load(dir.path().join("a.txt")).unwrap();
        let deck_b = Deck::load(dir.path().join("b.txt")).unwrap();

        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert(deck_a.cards[0].id(), 0);
        store.get_or_insert(deck_b.cards[0].id(), 0);
        store.set_deck_mastered(&deck_a.subject, 0);

        let n = reset_decks(&mut store, [&deck_a]).unwrap();
        assert_eq!(1, n);
        assert!(
            store.get(deck_a.cards[0].id()).is_none(),
            "a's schedule wiped"
        );
        assert!(
            store.get(deck_b.cards[0].id()).is_some(),
            "b's schedule intact"
        );
        assert!(!store.deck_mastered(&deck_a.subject));
    }

    /// A minimal virtual card belonging to `parent`, mirroring
    /// `tests/cli.rs`'s `sample_virtual_card`: its id is the `Card::id` of the
    /// card `parse(parent, text)` yields, identical to a deck card's.
    fn virtual_card(parent: &str, back: &str) -> crate::store::VirtualCard {
        let text = format!("# front\n\t{back}\n");
        let id = parser::parse_str(parent, &text).unwrap()[0].id();
        crate::store::VirtualCard {
            id,
            kind: crate::store::VirtualKind::Remediation,
            parent: parent.to_string(),
            text,
            created_ms: 0,
        }
    }

    #[test]
    fn resetting_a_deck_drops_its_virtual_cards_but_keeps_anothers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "# qa\n\tans-a\n").unwrap();
        let deck_a = Deck::load(dir.path().join("a.txt")).unwrap();

        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let vc_a = virtual_card("a.txt", "vc-a");
        let vc_other = virtual_card("other.txt", "vc-other");
        let (id_a, id_other) = (vc_a.id, vc_other.id);
        store.insert_virtual(vc_a);
        store.insert_virtual(vc_other);
        store.get_or_insert(id_a, 0);
        store.get_or_insert(id_other, 0);

        let n = reset_decks(&mut store, [&deck_a]).unwrap();
        assert_eq!(0, n, "no authored cards had progress");
        assert!(
            store.get_virtual(id_a).is_none(),
            "a's virtual card dropped"
        );
        assert!(store.get(id_a).is_none(), "a's virtual schedule dropped");
        assert!(
            store.get_virtual(id_other).is_some(),
            "another deck's virtual card survives"
        );
    }
}
