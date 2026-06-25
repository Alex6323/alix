//! The progress store.
//!
//! Progress is kept in a single JSON file (by default
//! `~/.local/share/alix/progress.json`), created on first save.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// How many of the most recent reviews are kept per card.
const HISTORY_CAP: usize = 50;

/// The highest Leitner stage. Cards that keep passing stay here.
pub const MAX_STAGE: u8 = 5;

/// The current on-disk store-format version. Bump it when the persisted shape
/// changes in a way an older binary couldn't safely read, and add the matching
/// step in [`migrate`].
const CURRENT_VERSION: u32 = 1;

/// One recorded review of a card.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Review {
    /// When the review happened (Unix ms).
    pub ts_ms: u64,
    /// Whether the card was answered correctly.
    pub passed: bool,
}

/// Scheduling state of the SM-2 algorithm for a single card.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Sm2State {
    /// The ease factor (>= 1.3).
    pub ease: f64,
    /// Number of successful repetitions in a row.
    pub reps: u32,
    /// The current inter-repetition interval in milliseconds.
    pub interval_ms: u64,
    /// When the card is due next (Unix ms).
    pub due_ms: u64,
}

/// The stored state of a single card, keyed by its identity hash.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CardState {
    /// The Leitner stage (1..=5). Kept up to date by both schedulers so the
    /// user can switch between them.
    pub stage: u8,
    /// When the card entered its current stage (Unix ms).
    pub stage_entered_ms: u64,
    /// SM-2 state; present once the card has been reviewed with the SM-2
    /// scheduler (or derived from the stage on first SM-2 review).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sm2: Option<Sm2State>,
    /// Total number of reviews.
    #[serde(default)]
    pub total_reviews: u32,
    /// Total number of passed reviews.
    #[serde(default)]
    pub total_passes: u32,
    /// Current streak of consecutive passes.
    #[serde(default)]
    pub streak: u32,
    /// The most recent reviews, oldest first (capped).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Review>,
}

impl CardState {
    /// State for a card entering the system now at stage 1.
    pub fn new(now_ms: u64) -> Self {
        Self {
            stage: 1,
            stage_entered_ms: now_ms,
            sm2: None,
            total_reviews: 0,
            total_passes: 0,
            streak: 0,
            history: Vec::new(),
        }
    }

    /// Appends a review to the bounded history and updates the counters.
    pub fn record_review(&mut self, ts_ms: u64, passed: bool) {
        self.total_reviews += 1;
        if passed {
            self.total_passes += 1;
            self.streak += 1;
        } else {
            self.streak = 0;
        }
        self.history.push(Review { ts_ms, passed });
        if self.history.len() > HISTORY_CAP {
            let excess = self.history.len() - HISTORY_CAP;
            self.history.drain(..excess);
        }
    }
}

/// Deck-level progress, keyed by deck subject (= file name). Currently records
/// that the deck's AI exam has been passed ("mastered").
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeckProgress {
    /// When the exam was last passed (Unix ms).
    pub mastered_at_ms: u64,
}

/// On-disk representation of the store.
#[derive(Serialize, Deserialize)]
struct StoreFile {
    /// Format version. Defaults to 1 for a file written before the field was
    /// required, so a legacy store still loads. [`migrate`] checks it.
    #[serde(default = "default_version")]
    version: u32,
    /// Card states keyed by the decimal string of the card's identity hash
    /// (JSON object keys must be strings).
    cards: HashMap<String, CardState>,
    /// Deck-level progress keyed by subject. Optional: a store written before
    /// this field existed (or with no mastered decks) simply has no `decks`
    /// key.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    decks: HashMap<String, DeckProgress>,
}

/// The progress store for all decks.
pub struct Store {
    path: PathBuf,
    cards: HashMap<u64, CardState>,
    decks: HashMap<String, DeckProgress>,
}

/// An error loading or saving the store.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Format {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "{path}: progress file is version {found}, but this build of alix \
         understands only up to {supported} — upgrade alix to open it"
    )]
    TooNew {
        path: PathBuf,
        found: u32,
        supported: u32,
    },
}

impl Store {
    /// Opens the store at `path`, creating an empty in-memory one if the file
    /// does not exist yet (it is written on the first [`save`](Self::save)).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();

        if !path.exists() {
            return Ok(Self {
                path,
                cards: HashMap::new(),
                decks: HashMap::new(),
            });
        }

        let text = std::fs::read_to_string(&path).map_err(|source| StoreError::Io {
            path: path.clone(),
            source,
        })?;
        let file: StoreFile = serde_json::from_str(&text).map_err(|source| StoreError::Format {
            path: path.clone(),
            source,
        })?;
        let file = migrate(file, &path)?;
        let mut cards = HashMap::with_capacity(file.cards.len());
        for (key, state) in file.cards {
            let hash = key.parse::<u64>().map_err(|e| StoreError::Format {
                path: path.clone(),
                source: serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("bad card key {key:?}: {e}"),
                )),
            })?;
            cards.insert(hash, state);
        }
        Ok(Self {
            path,
            cards,
            decks: file.decks,
        })
    }

    /// Saves the store atomically (write to a temp file, then rename).
    pub fn save(&self) -> Result<(), StoreError> {
        let io_err = |source| StoreError::Io {
            path: self.path.clone(),
            source,
        };

        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(io_err)?;
        }

        let file = StoreFile {
            version: CURRENT_VERSION,
            cards: self
                .cards
                .iter()
                .map(|(hash, state)| (hash.to_string(), state.clone()))
                .collect(),
            decks: self.decks.clone(),
        };
        let json = serde_json::to_string_pretty(&file).map_err(|source| StoreError::Format {
            path: self.path.clone(),
            source,
        })?;

        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(io_err)?;
        std::fs::rename(&tmp, &self.path).map_err(io_err)?;
        Ok(())
    }

    /// Returns the state of a card, if it has been seen before.
    pub fn get(&self, card_id: u64) -> Option<&CardState> {
        self.cards.get(&card_id)
    }

    /// The most recent review timestamp across all cards (Unix ms), if any —
    /// when progress was last made in this store. Reflects actual reviews, not
    /// merely opening a deck.
    pub fn last_review_ms(&self) -> Option<u64> {
        self.cards
            .values()
            .filter_map(|state| state.history.last().map(|review| review.ts_ms))
            .max()
    }

    /// Returns a mutable reference to the state of a card, inserting a fresh
    /// stage-1 state if the card is new.
    pub fn get_or_insert(&mut self, card_id: u64, now_ms: u64) -> &mut CardState {
        self.cards
            .entry(card_id)
            .or_insert_with(|| CardState::new(now_ms))
    }

    /// Drops a card's stored state, e.g. when the card is deleted from its
    /// deck. Returns whether an entry was present. Does not save.
    pub fn remove(&mut self, card_id: u64) -> bool {
        self.cards.remove(&card_id).is_some()
    }

    /// Whether the given deck has passed its AI exam ("mastered").
    pub fn deck_mastered(&self, subject: &str) -> bool {
        self.decks.contains_key(subject)
    }

    /// When the deck was mastered (epoch ms), if it has been.
    pub fn deck_mastered_at(&self, subject: &str) -> Option<u64> {
        self.decks.get(subject).map(|d| d.mastered_at_ms)
    }

    /// Records that the deck passed its exam at `now_ms`. Does not save.
    pub fn set_deck_mastered(&mut self, subject: &str, now_ms: u64) {
        self.decks.insert(
            subject.to_string(),
            DeckProgress {
                mastered_at_ms: now_ms,
            },
        );
    }

    /// Drops a deck's mastered state (e.g. on per-deck reset). Returns whether
    /// an entry was present. Does not save.
    pub fn clear_deck_mastered(&mut self, subject: &str) -> bool {
        self.decks.remove(subject).is_some()
    }

    /// Clears all stored progress, returning how many cards were removed (e.g.
    /// for `alix reset --all`). Also drops all deck-mastered state. Does not
    /// save.
    pub fn clear(&mut self) -> usize {
        let n = self.cards.len();
        self.cards.clear();
        self.decks.clear();
        n
    }

    /// The number of cards tracked by this store.
    pub fn len(&self) -> usize {
        self.cards.len()
    }

    /// Returns `true` if no cards are tracked.
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// The path of the store file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Serde default for a legacy store with no `version` field: the oldest format.
fn default_version() -> u32 {
    1
}

/// Brings a just-loaded [`StoreFile`] up to [`CURRENT_VERSION`], or fails loudly
/// if it was written by a newer alix than this one. Refusing is the safety net:
/// silently saving a newer store back at the current version would drop whatever
/// fields the newer format added — i.e. quietly wipe progress. Future migrations
/// step the version up one at a time here, oldest first.
fn migrate(file: StoreFile, path: &Path) -> Result<StoreFile, StoreError> {
    if file.version > CURRENT_VERSION {
        return Err(StoreError::TooNew {
            path: path.to_path_buf(),
            found: file.version,
            supported: CURRENT_VERSION,
        });
    }
    // e.g. `if file.version < 2 { file = v1_to_v2(file); }`
    Ok(file)
}

/// The default location of the store file
/// (`~/.local/share/alix/progress.json` on Linux).
pub fn default_store_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "alix").map(|dirs| dirs.data_dir().join("progress.json"))
}

/// One-time adoption of a pre-rename `flash` data directory. The tool used to
/// store progress under `flash/`; if the new `alix/` data dir doesn't exist yet
/// but the legacy `flash/` one does, move it across so existing progress
/// survives the rename. Best-effort — any error is ignored (a failed move just
/// means the user starts fresh, never a crash). Call once at startup.
pub fn migrate_legacy_data_dir() {
    let old = directories::ProjectDirs::from("", "", "flash").map(|d| d.data_dir().to_path_buf());
    let new = directories::ProjectDirs::from("", "", "alix").map(|d| d.data_dir().to_path_buf());
    if let (Some(old), Some(new)) = (old, new) {
        adopt_legacy_dir(&old, &new);
    }
}

/// Renames `old` to `new` when `new` is absent and `old` exists. Split out from
/// [`migrate_legacy_data_dir`] so it can be tested without touching the real
/// platform data directory.
fn adopt_legacy_dir(old: &Path, new: &Path) {
    if new.exists() || !old.exists() {
        return;
    }
    if let Some(parent) = new.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::rename(old, new);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let store = Store::open(&path).unwrap();
        assert!(store.is_empty());
    }

    #[test]
    fn adopt_legacy_dir_moves_when_new_is_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("flash");
        let new = tmp.path().join("alix");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("progress.json"), "{}").unwrap();
        adopt_legacy_dir(&old, &new);
        assert!(
            new.join("progress.json").exists(),
            "progress should have moved"
        );
        assert!(
            !old.exists(),
            "the legacy dir should be gone after the move"
        );
    }

    #[test]
    fn adopt_legacy_dir_leaves_an_existing_new_dir_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("flash");
        let new = tmp.path().join("alix");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("progress.json"), "OLD").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("progress.json"), "NEW").unwrap();
        adopt_legacy_dir(&old, &new);
        assert_eq!(
            "NEW",
            std::fs::read_to_string(new.join("progress.json")).unwrap()
        );
        assert!(old.exists(), "the legacy dir should be left in place");
    }

    #[test]
    fn open_rejects_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(&path, "this is not json").unwrap();
        assert!(Store::open(&path).is_err());
    }

    #[test]
    fn open_rejects_a_non_numeric_card_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        // A valid StoreFile shape, but a card key that isn't a u64 hash.
        std::fs::write(
            &path,
            r#"{"version":1,"cards":{"not-a-number":{"stage":1,"stage_entered_ms":0}}}"#,
        )
        .unwrap();
        let err = Store::open(&path).err().unwrap();
        assert!(format!("{err}").contains("bad card key"));
    }

    #[test]
    fn last_review_ms_is_the_latest_across_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut store = Store::open(&path).unwrap();
        assert_eq!(None, store.last_review_ms());
        store.get_or_insert(1, 0).record_review(100, true);
        store.get_or_insert(2, 0).record_review(300, true);
        assert_eq!(Some(300), store.last_review_ms());
    }

    #[test]
    fn path_returns_the_store_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let store = Store::open(&path).unwrap();
        assert_eq!(path.as_path(), store.path());
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");

        let mut store = Store::open(&path).unwrap();
        let state = store.get_or_insert(42, 1000);
        state.stage = 3;
        state.record_review(1000, true);
        store.save().unwrap();

        let reloaded = Store::open(&path).unwrap();
        assert_eq!(1, reloaded.len());
        let state = reloaded.get(42).unwrap();
        assert_eq!(3, state.stage);
        assert_eq!(1, state.total_reviews);
        assert_eq!(
            vec![Review {
                ts_ms: 1000,
                passed: true
            }],
            state.history
        );
    }

    #[test]
    fn deck_mastered_roundtrips_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");

        let mut store = Store::open(&path).unwrap();
        assert!(!store.deck_mastered("rust.txt"));
        assert_eq!(None, store.deck_mastered_at("rust.txt"));
        store.set_deck_mastered("rust.txt", 1234);
        assert!(store.deck_mastered("rust.txt"));
        assert_eq!(Some(1234), store.deck_mastered_at("rust.txt"));
        store.save().unwrap();

        // Survives a save/reload.
        let mut reloaded = Store::open(&path).unwrap();
        assert!(reloaded.deck_mastered("rust.txt"));
        assert_eq!(Some(1234), reloaded.deck_mastered_at("rust.txt"));
        // Per-deck clear drops just that deck.
        assert!(reloaded.clear_deck_mastered("rust.txt"));
        assert!(!reloaded.deck_mastered("rust.txt"));
        assert!(!reloaded.clear_deck_mastered("rust.txt")); // nothing left
    }

    #[test]
    fn clear_also_drops_deck_mastered() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.set_deck_mastered("a.txt", 1);
        store.clear();
        assert!(!store.deck_mastered("a.txt"));
    }

    #[test]
    fn loads_store_file_without_decks_field() {
        // A store written before the `decks` field existed must still load.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(&path, "{\"version\":1,\"cards\":{}}").unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.is_empty());
        assert!(!store.deck_mastered("anything.txt"));
    }

    #[test]
    fn loads_a_store_file_without_a_version_field() {
        // A file predating the `version` field defaults to v1 and still loads.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(&path, "{\"cards\":{}}").unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.is_empty());
    }

    #[test]
    fn rejects_a_store_from_a_newer_alix_without_touching_it() {
        // A store written by a future alix (higher version) must NOT be
        // silently downgraded and re-saved — that would drop fields the newer
        // format added. Open must fail and leave the file exactly as it was.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let newer = "{\"version\":999,\"cards\":{}}";
        std::fs::write(&path, newer).unwrap();

        let err = Store::open(&path).err().unwrap();
        assert!(format!("{err}").contains("version 999"));
        assert_eq!(newer, std::fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn save_writes_the_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut store = Store::open(&path).unwrap();
        store.get_or_insert(1, 0);
        store.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(&format!("\"version\": {CURRENT_VERSION}")));
    }

    #[test]
    fn remove_drops_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert(42, 1000);
        assert!(store.remove(42));
        assert!(store.get(42).is_none());
        // Removing again reports nothing was there.
        assert!(!store.remove(42));
    }

    #[test]
    fn clear_empties_and_counts() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert(1, 0);
        store.get_or_insert(2, 0);
        assert_eq!(2, store.clear());
        assert!(store.is_empty());
        assert_eq!(0, store.clear()); // already empty
    }

    #[test]
    fn history_is_capped() {
        let mut state = CardState::new(0);
        for i in 0..(HISTORY_CAP as u64 + 10) {
            state.record_review(i, true);
        }
        assert_eq!(HISTORY_CAP, state.history.len());
        assert_eq!(10, state.history[0].ts_ms);
        assert_eq!(HISTORY_CAP as u32 + 10, state.total_reviews);
    }

    #[test]
    fn streak_resets_on_fail() {
        let mut state = CardState::new(0);
        state.record_review(1, true);
        state.record_review(2, true);
        assert_eq!(2, state.streak);
        state.record_review(3, false);
        assert_eq!(0, state.streak);
        assert_eq!(2, state.total_passes);
        assert_eq!(3, state.total_reviews);
    }
}
