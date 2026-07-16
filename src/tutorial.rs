//! The bundled tutorial deck and its one seeding rule. A brand-new decks
//! folder gets "The alix tutorial" (a deck that teaches alix while being
//! reviewed); an existing folder is never touched, and a deleted tutorial
//! never comes back — deleting it is the graduation.

use std::path::Path;

/// The tutorial deck's file name inside a decks folder. The mobile app seeds
/// its own bundled copy under the same name, so progress hashes agree.
pub const TUTORIAL_FILE: &str = "tutorial.txt";

/// The tutorial deck, embedded verbatim from `assets/decks/tutorial.txt`.
pub const TUTORIAL_DECK: &str = include_str!("../assets/decks/tutorial.txt");

/// Seeds the tutorial into `dir` **only when `dir` itself does not exist
/// yet** — the one moment we know this is a first run. An existing folder
/// (even an empty one) is left alone: seeding into it could surprise a user
/// who made the folder deliberately, and re-seeding after a delete would
/// undo the tutorial's own "delete me" graduation. Best-effort: any error is
/// reported as `false`, never a crash — the picker simply starts empty.
pub fn seed_new_decks_dir(dir: &Path) -> bool {
    if dir.exists() {
        return false;
    }
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    std::fs::write(dir.join(TUTORIAL_FILE), TUTORIAL_DECK).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_decks_dir_is_created_and_seeded() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("decks");
        assert!(seed_new_decks_dir(&dir));
        let seeded = std::fs::read_to_string(dir.join(TUTORIAL_FILE)).unwrap();
        assert_eq!(TUTORIAL_DECK, seeded);
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

    /// The mobile app bundles its own copy (Flutter assets cannot reach
    /// outside the app package); this pins the two files together. Skipped
    /// when the mobile tree is absent (the published crate ships src/ and
    /// assets/ only), stated loudly so the skip is never mistaken for a pass.
    #[test]
    fn the_mobile_copy_matches_the_canonical_deck() {
        let mobile = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("apps/mobile/assets/decks/tutorial.txt");
        if !mobile.exists() {
            eprintln!("skipping: no apps/mobile tree here (published crate)");
            return;
        }
        let copy = std::fs::read_to_string(mobile).unwrap();
        assert_eq!(
            TUTORIAL_DECK, copy,
            "apps/mobile/assets/decks/tutorial.txt drifted from \
             assets/decks/tutorial.txt — copy the canonical file over"
        );
    }
}
