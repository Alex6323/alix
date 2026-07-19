use std::path::Path;

pub const TUTORIAL_FILE: &str = "tutorial.md";

// Deliberately unstamped: seeding mints each install's own identity tokens.
pub const TUTORIAL_DECK: &str = include_str!("../assets/decks/tutorial.md");

// An existing folder (even empty) is left alone: re-seeding would undo the
// tutorial's own delete-to-graduate design.
pub fn seed_new_decks_dir(dir: &Path) -> bool {
    if dir.exists() {
        return false;
    }
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let path = dir.join(TUTORIAL_FILE);
    if std::fs::write(&path, TUTORIAL_DECK).is_err() {
        return false;
    }
    // Best-effort: a failed stamp leaves an unstamped but still-loadable
    // tutorial; review-open stamps it later.
    let _ = crate::stamp::stamp_deck(&path);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_decks_dir_is_created_and_seeded_stamped() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("decks");
        assert!(seed_new_decks_dir(&dir));
        let seeded = std::fs::read_to_string(dir.join(TUTORIAL_FILE)).unwrap();
        let asset = crate::l1::parse_l1(TUTORIAL_FILE, TUTORIAL_DECK).unwrap();
        assert!(asset.deck_token.is_none(), "the asset stays unstamped");
        let deck = crate::l1::parse_l1(TUTORIAL_FILE, &seeded).unwrap();
        assert!(deck.deck_token.is_some(), "seeding mints a deck id");
        assert_eq!(asset.cards.len(), deck.cards.len());
        assert!(deck.cards.iter().all(|c| c.id().is_some()));
    }

    #[test]
    fn an_existing_dir_is_never_seeded_even_when_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("decks");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!seed_new_decks_dir(&dir));
        assert!(!dir.join(TUTORIAL_FILE).exists());
    }

    #[test]
    fn a_deleted_tutorial_stays_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("decks");
        assert!(seed_new_decks_dir(&dir));
        std::fs::remove_file(dir.join(TUTORIAL_FILE)).unwrap();
        assert!(!seed_new_decks_dir(&dir), "graduation must be final");
        assert!(!dir.join(TUTORIAL_FILE).exists());
    }

    #[test]
    fn the_tutorial_deck_parses_clean_with_cards() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(TUTORIAL_FILE);
        std::fs::write(&path, TUTORIAL_DECK).unwrap();
        let deck = crate::deck::Deck::load(&path).unwrap();
        assert!(
            deck.cards.len() >= 10,
            "the tutorial should be a real deck, found {}",
            deck.cards.len()
        );
    }

    // Skipped without apps/mobile (published crate ships src/+assets/ only);
    // stated loudly so a skip is never mistaken for a pass.
    #[test]
    fn the_mobile_copy_matches_the_canonical_deck() {
        let mobile_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("apps/mobile");
        if !mobile_dir.exists() {
            eprintln!("skipping: no apps/mobile tree here (published crate)");
            return;
        }
        let copy = std::fs::read_to_string(mobile_dir.join("assets/decks/tutorial.md")).unwrap();
        assert_eq!(
            TUTORIAL_DECK, copy,
            "apps/mobile/assets/decks/tutorial.md drifted from \
             assets/decks/tutorial.md (copy the canonical file over)"
        );
    }
}
