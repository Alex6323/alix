use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::{
    augment::{self, AugmentCache},
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
    #[error(transparent)]
    Augment(#[from] augment::AugmentError),
    #[error("{path}: deck is not initialized")]
    MissingDeckId { path: PathBuf },
    #[error("{path}: expected a progress document or directory")]
    InvalidStorePath { path: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub root: PathBuf,
    pub progress: PathBuf,
    pub augment: PathBuf,
}

impl Layout {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        Self {
            progress: root.join("progress"),
            augment: root.join("augment"),
            root,
        }
    }

    pub fn progress_for(&self, deck_id: &str) -> PathBuf {
        self.progress.join(format!("{deck_id}.json"))
    }

    pub fn augment_for(&self, deck_id: &str) -> PathBuf {
        self.augment.join(format!("{deck_id}.json"))
    }
}

pub fn deck_id_from_document(path: &Path) -> Option<&str> {
    path.file_name()?
        .to_str()?
        .strip_suffix(".json")
        .filter(|id| !id.is_empty())
}

pub fn open_store(deck_path: &Path, state_root: &Path) -> Result<Store, StateError> {
    let (layout, deck) = prepare(deck_path, state_root)?;
    let deck_id = require_deck_id(&deck, deck_path)?;
    let mut store = Store::open_deck(
        layout.progress_for(deck_id),
        deck_id.to_string(),
        deck.subject,
    )?;
    store.device = store::device_label();
    Ok(store)
}

pub fn open_stores(deck_paths: &[PathBuf], state_root: &Path) -> Result<Store, StateError> {
    if let [deck_path] = deck_paths {
        return open_store(deck_path, state_root);
    }
    let decks = deck_paths
        .iter()
        .map(Deck::load)
        .collect::<Result<Vec<_>, _>>()?;
    let mut store = Store::open_for_decks(Layout::new(state_root).progress, &decks)?;
    store.device = store::device_label();
    Ok(store)
}

pub fn open_aggregate_store(state_root: &Path) -> Result<Store, StateError> {
    let mut store = Store::open(Layout::new(state_root).progress)?;
    store.device = store::device_label();
    Ok(store)
}

pub fn open_augment(deck_path: &Path, state_root: &Path) -> Result<AugmentCache, StateError> {
    let (layout, deck) = prepare(deck_path, state_root)?;
    let deck_id = require_deck_id(&deck, deck_path)?;
    Ok(AugmentCache::open_deck(
        layout.augment_for(deck_id),
        deck_id,
    )?)
}

pub fn open_augment_read_only(
    deck_id: &str,
    state_root: &Path,
) -> Result<AugmentCache, StateError> {
    Ok(AugmentCache::open_deck(
        Layout::new(state_root).augment_for(deck_id),
        deck_id,
    )?)
}

pub fn prepare(deck_path: &Path, state_root: &Path) -> Result<(Layout, Deck), StateError> {
    let deck = Deck::load(deck_path)?;
    require_deck_id(&deck, deck_path)?;
    Ok((Layout::new(state_root), deck))
}

pub fn retire_replaced_deck(store_path: &Path, deck_id: &str) -> Result<bool, StateError> {
    let progress = progress_document_for(store_path, deck_id)?;
    let augment = augment::augment_path_for(&progress);
    if progress.is_file() {
        let (_, _, data) = store::read_deck_data(&progress, deck_id, None)?;
        if !data.cards.is_empty()
            || !data.records.is_empty()
            || !data.decks.is_empty()
            || !data.virtual_cards.is_empty()
        {
            return Ok(false);
        }
    }
    if augment.is_file() {
        let (_, data) = augment::read_deck_data(&augment, deck_id)?;
        if !data.cards.is_empty() || !data.topologies.is_empty() {
            return Ok(false);
        }
        std::fs::remove_file(&augment).map_err(|source| StateError::Io {
            path: augment,
            source,
        })?;
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

fn progress_document_for(store_path: &Path, deck_id: &str) -> Result<PathBuf, StateError> {
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
    use super::*;

    fn deck(path: &Path, deck_id: &str, card_id: &str) {
        std::fs::write(
            path,
            format!(
                "---\nalix-id: \"{deck_id}\"\n---\n## question <!-- id: {card_id} -->\nanswer\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn a_state_root_addresses_both_documents_by_deck_id() {
        let layout = Layout::new("/data/alix");
        assert_eq!(
            Path::new("/data/alix/progress/deck1.json"),
            layout.progress_for("deck1")
        );
        assert_eq!(
            Path::new("/data/alix/augment/deck1.json"),
            layout.augment_for("deck1")
        );
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
        store.get_or_insert("card1", 1);
        store.save().unwrap();

        assert_eq!(
            dir.path().join("progress/deck1.json"),
            store.path().to_path_buf()
        );
        assert!(dir.path().join("progress/deck1.json").is_file());
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
        store.get_or_insert("card1", 1);
        store.set_deck_mastered("old.md", 1);
        store.save().unwrap();
        std::fs::rename(&old_path, &new_path).unwrap();

        let renamed = open_store(&new_path, dir.path()).unwrap();

        assert_eq!(dir.path().join("progress/deck1.json"), renamed.path());
        assert!(renamed.get("card1").is_some());
        assert!(renamed.deck_mastered("new.md"));
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
        aggregate.get_or_insert("card1", 1);
        aggregate.get_or_insert("card2", 1);
        aggregate.save().unwrap();
        let first = dir.path().join("progress/deck1.json");
        let second = dir.path().join("progress/deck2.json");
        let second_before = std::fs::read(&second).unwrap();

        aggregate.remove("card1");
        aggregate.save().unwrap();

        assert_ne!(std::fs::read(first).unwrap(), second_before);
        assert_eq!(std::fs::read(second).unwrap(), second_before);
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
}
