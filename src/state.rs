use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::{
    deck::{Deck, DeckError},
    store::{self, Store, StoreError},
};

#[derive(Debug, Error)]
pub enum StateError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Deck(#[from] DeckError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("{path}: deck is not initialized")]
    MissingDeckId { path: PathBuf },
    #[error("{path}: expected a progress document or directory")]
    InvalidStorePath { path: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserFiles {
    root: PathBuf,
}

impl UserFiles {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn progress(&self) -> PathBuf {
        self.root.join("progress")
    }

    pub fn progress_for(&self, deck_id: &str) -> PathBuf {
        self.progress().join(format!("{deck_id}.json"))
    }

    pub fn recent(&self) -> PathBuf {
        self.root.join("recent.json")
    }

    pub fn local_manifest(&self) -> PathBuf {
        self.root.join(crate::config::LOCAL_MANIFEST)
    }
}

pub fn deck_id_from_document(path: &Path) -> Option<&str> {
    path.file_name()?
        .to_str()?
        .strip_suffix(".json")
        .filter(|id| !id.is_empty())
}

pub fn open_store(deck_path: &Path, user_root: &Path) -> Result<Store, StateError> {
    let (files, deck, deck_id) = prepare(deck_path, user_root)?;
    let mut store = Store::open_deck(files.progress_for(&deck_id), deck_id, deck.subject)?;
    store.device = store::device_label();
    Ok(store)
}

pub fn open_stores(deck_paths: &[PathBuf], user_root: &Path) -> Result<Store, StateError> {
    if let [deck_path] = deck_paths {
        return open_store(deck_path, user_root);
    }
    let decks = deck_paths
        .iter()
        .map(Deck::load)
        .collect::<Result<Vec<_>, _>>()?;
    let mut store = Store::open_for_decks(UserFiles::new(user_root).progress(), &decks)?;
    store.device = store::device_label();
    Ok(store)
}

pub fn open_aggregate_store(user_root: &Path) -> Result<Store, StateError> {
    let mut store = Store::open(UserFiles::new(user_root).progress())?;
    store.device = store::device_label();
    Ok(store)
}

/// The viewing-side open (listings, the server boot): per-document read
/// granularity, failed deck ids collected into the store instead of failing
/// it (see Store::open_aggregate_tolerant).
pub fn open_aggregate_store_tolerant(user_root: &Path) -> Result<Store, StateError> {
    let mut store = Store::open_aggregate_tolerant(UserFiles::new(user_root).progress())?;
    store.device = store::device_label();
    Ok(store)
}

pub fn prepare(
    deck_path: &Path,
    user_root: &Path,
) -> Result<(UserFiles, Deck, String), StateError> {
    let deck = Deck::load(deck_path)?;
    let deck_id = require_deck_id(&deck, deck_path)?.to_string();
    Ok((UserFiles::new(user_root), deck, deck_id))
}

pub fn retire_replaced_progress(store_path: &Path, deck_id: &str) -> Result<bool, StateError> {
    let progress = progress_document_for(store_path, deck_id)?;
    if progress.is_file() {
        let (_, _, data) = store::read_deck_data(&progress, deck_id, None)?;
        if !data.cards.is_empty() || !data.records.is_empty() || !data.deck.is_empty() {
            return Ok(false);
        }
    }
    if progress.is_file() {
        std::fs::remove_file(&progress).map_err(|source| StateError::Io {
            path: progress,
            source,
        })?;
    }
    Ok(true)
}

fn require_deck_id<'a>(deck: &'a Deck, path: &Path) -> Result<&'a str, StateError> {
    deck.deck_token
        .as_deref()
        .ok_or_else(|| StateError::MissingDeckId {
            path: path.to_path_buf(),
        })
}

pub(crate) fn progress_document_for(
    store_path: &Path,
    deck_id: &str,
) -> Result<PathBuf, StateError> {
    if store_path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "progress")
        && store_path
            .extension()
            .is_some_and(|extension| extension == "json")
    {
        let progress = store_path
            .parent()
            .ok_or_else(|| StateError::InvalidStorePath {
                path: store_path.to_path_buf(),
            })?;
        return Ok(progress.join(format!("{deck_id}.json")));
    }
    if store_path
        .file_name()
        .is_some_and(|name| name == "progress")
    {
        return Ok(store_path.join(format!("{deck_id}.json")));
    }
    Err(StateError::InvalidStorePath {
        path: store_path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn deck(path: &Path, deck_id: &str, card_id: &str) {
        std::fs::write(
            path,
            format!(
                "---\nformat-version: 1\nid: \"deck-{deck_id}\"\n---\n## question\nanswer\n<!-- id: card-{card_id} -->\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn user_files_address_private_documents() {
        let files = UserFiles::new("/data/alix");
        assert_eq!(
            Path::new("/data/alix/progress/deck1.json"),
            files.progress_for("deck1")
        );
        assert_eq!(Path::new("/data/alix/recent.json"), files.recent());
    }

    #[test]
    fn document_file_names_round_trip_deck_ids() {
        assert_eq!(
            Some("01abc"),
            deck_id_from_document(Path::new("/decks/progress/01abc.json"))
        );
        assert_eq!(
            None,
            deck_id_from_document(Path::new("/decks/progress/.json"))
        );
        assert_eq!(
            None,
            deck_id_from_document(Path::new("/decks/progress/01abc"))
        );
    }

    #[test]
    fn saving_a_deck_creates_only_its_progress_document() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("deck.md");
        deck(&deck_path, "deck1", "card1");

        let mut store = open_store(&deck_path, dir.path()).unwrap();
        store.get_or_insert("card-card1");
        store.save().unwrap();

        assert_eq!(
            dir.path().join("progress/deck-deck1.json"),
            store.path().to_path_buf()
        );
        assert!(dir.path().join("progress/deck-deck1.json").is_file());
        assert_eq!(
            vec![dir.path().join("deck.md"), dir.path().join("progress")],
            {
                let mut entries = std::fs::read_dir(dir.path())
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect::<Vec<_>>();
                entries.sort();
                entries
            }
        );
    }

    #[test]
    fn renaming_a_deck_keeps_its_identity_derived_progress_document() {
        let dir = tempfile::tempdir().unwrap();
        let old_path = dir.path().join("old.md");
        let new_path = dir.path().join("new.md");
        deck(&old_path, "deck1", "card1");
        let mut store = open_store(&old_path, dir.path()).unwrap();
        store.get_or_insert("card-card1");
        store.set_deck_mastered("deck-deck1", 1);
        store.save().unwrap();
        std::fs::rename(&old_path, &new_path).unwrap();

        let renamed = open_store(&new_path, dir.path()).unwrap();

        assert_eq!(dir.path().join("progress/deck-deck1.json"), renamed.path());
        assert!(renamed.get("card-card1").is_some());
        assert!(renamed.deck_mastered("deck-deck1"));
    }

    #[test]
    fn aggregate_open_finds_deck_level_state_by_deck_id_after_a_rename() {
        // The prior bug: doctor/`reset --orphans` scan through
        // `open_aggregate_store`, which (unlike `open_store`) passes no
        // `current_subject` hint, so a subject-keyed rebind never fires
        // there. Deck-level state and orphan detection must key off the
        // stable deck id, never off the filename, or a plain rename
        // orphans the deck's mastery/badge/last-depth state.
        let dir = tempfile::tempdir().unwrap();
        let old_path = dir.path().join("old.md");
        deck(&old_path, "deck1", "card1");
        let mut store = open_store(&old_path, dir.path()).unwrap();
        store.set_deck_mastered("deck-deck1", 1);
        store.set_exam_failed("deck-deck1", 42);
        store.set_last_depth("deck-deck1", crate::depth::Depth::Recall);
        store.save().unwrap();

        let new_path = dir.path().join("new.md");
        std::fs::rename(&old_path, &new_path).unwrap();

        let aggregate = open_aggregate_store(dir.path()).unwrap();
        assert!(
            aggregate.deck_mastered("deck-deck1"),
            "deck-level state must survive a rename, found by the deck id"
        );
        assert_eq!(
            Some(42),
            aggregate.exam_failed_at("deck-deck1"),
            "exam-failed state must survive a rename, found by the deck id"
        );
        assert_eq!(
            Some(crate::depth::Depth::Recall),
            aggregate.last_depth("deck-deck1"),
            "last-depth state must survive a rename, found by the deck id"
        );

        let known_deck_ids: HashSet<String> = std::iter::once("deck-deck1".to_string()).collect();
        let orphans = aggregate.orphans(&HashSet::new(), &known_deck_ids);
        assert!(
            orphans.decks.is_empty(),
            "a renamed-but-still-known deck must not report as an orphan: {orphans:?}"
        );
    }

    #[test]
    fn aggregate_progress_saves_only_the_changed_owning_document() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("first.md");
        let second_path = dir.path().join("second.md");
        deck(&first_path, "deck1", "card1");
        deck(&second_path, "deck2", "card2");
        let paths = [first_path, second_path];
        let mut aggregate = open_stores(&paths, dir.path()).unwrap();
        aggregate.get_or_insert("card-card1");
        aggregate.get_or_insert("card-card2");
        aggregate.save().unwrap();
        let first = dir.path().join("progress/deck-deck1.json");
        let second = dir.path().join("progress/deck-deck2.json");
        let second_before = std::fs::read(&second).unwrap();

        aggregate.remove("card-card1");
        aggregate.save().unwrap();

        assert_ne!(std::fs::read(first).unwrap(), second_before);
        assert_eq!(std::fs::read(second).unwrap(), second_before);
    }

    #[test]
    fn a_corrupt_sibling_document_never_blocks_opening_for_other_decks() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("first.md");
        let second_path = dir.path().join("second.md");
        deck(&first_path, "deck1", "card1");
        deck(&second_path, "deck2", "card2");
        let paths = [first_path, second_path];
        let mut aggregate = open_stores(&paths, dir.path()).unwrap();
        aggregate.get_or_insert("card-card1");
        aggregate.save().unwrap();
        std::fs::write(dir.path().join("progress/deck-other.json"), "{ corrupt").unwrap();

        let reopened = open_stores(&paths, dir.path()).unwrap();
        assert!(
            reopened.get("card-card1").is_some(),
            "the expected decks' progress must survive a damaged sibling"
        );
        assert!(
            reopened.progress_error("deck-other"),
            "the skipped sibling must be reported, not silently absent"
        );
        assert!(!reopened.progress_error("deck-deck1"));
    }

    #[test]
    fn tolerance_never_extends_to_the_decks_the_open_is_for() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("first.md");
        let second_path = dir.path().join("second.md");
        deck(&first_path, "deck1", "card1");
        deck(&second_path, "deck2", "card2");
        let paths = [first_path, second_path];
        let mut aggregate = open_stores(&paths, dir.path()).unwrap();
        aggregate.get_or_insert("card-card1");
        aggregate.save().unwrap();
        std::fs::write(dir.path().join("progress/deck-deck1.json"), "{ corrupt").unwrap();

        let error = match open_stores(&paths, dir.path()) {
            Ok(_) => panic!("an expected deck's own corrupt document must fail the open"),
            Err(error) => error,
        };
        match error {
            StateError::Store(crate::store::StoreError::Format { path, .. }) => {
                assert!(path.ends_with("deck-deck1.json"), "{}", path.display());
            }
            other => panic!("expected a Format error naming the document, got {other:?}"),
        }
    }

    #[test]
    fn a_deck_without_an_identity_cannot_create_state() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("deck.md");
        std::fs::write(&deck_path, "## question\nanswer\n").unwrap();

        let error = match open_store(&deck_path, dir.path()) {
            Ok(_) => panic!("an uninitialized deck opened state"),
            Err(error) => error,
        };

        assert!(matches!(error, StateError::MissingDeckId { .. }));
        assert!(!dir.path().join("progress").exists());
    }

    #[test]
    fn progress_with_any_single_kind_of_state_is_never_retired() {
        let filled = [
            ("cards", r#""cards":{"card-r1":{}}"#),
            (
                "records",
                r#""cards":{},"records":{"card-r1":{"version":1,"holes":[]}}"#,
            ),
            ("deck", r#""cards":{},"deck":{"last_depth":"recall"}"#),
        ];
        for (kind, body) in filled {
            let dir = tempfile::tempdir().unwrap();
            let progress = dir.path().join("progress");
            std::fs::create_dir(&progress).unwrap();
            let doc = progress.join("deck-r.json");
            std::fs::write(
                &doc,
                format!(
                    r#"{{"version":1,"deck_id":"deck-r","subject":"r.md","revision":1,{body}}}"#
                ),
            )
            .unwrap();
            let retired = retire_replaced_progress(&doc, "deck-r").unwrap();
            assert!(!retired, "{kind}-bearing progress must be kept");
            assert!(doc.is_file(), "{kind}: the document must stay in place");
        }
    }

    #[test]
    fn progress_document_files_require_both_the_progress_parent_and_json_extension() {
        let root = Path::new("/decks");
        assert_eq!(
            root.join("progress/deck-next.json"),
            progress_document_for(&root.join("progress/deck-current.json"), "deck-next").unwrap()
        );
        assert_eq!(
            root.join("progress/deck-next.json"),
            progress_document_for(&root.join("progress"), "deck-next").unwrap()
        );
        assert!(matches!(
            progress_document_for(&root.join("progress/deck-current.txt"), "deck-next"),
            Err(StateError::InvalidStorePath { .. })
        ));
        assert!(matches!(
            progress_document_for(&root.join("other/deck-current.json"), "deck-next"),
            Err(StateError::InvalidStorePath { .. })
        ));
    }
}
