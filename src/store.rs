//! The progress store.
//!
//! Progress is kept in a single JSON file (by default
//! `~/.local/share/flash/progress.json`), created on first save.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// How many of the most recent reviews are kept per card.
const HISTORY_CAP: usize = 50;

/// The highest Leitner stage. Cards that keep passing stay here.
pub const MAX_STAGE: u8 = 5;

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

/// On-disk representation of the store.
#[derive(Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    /// Card states keyed by the decimal string of the card's identity hash
    /// (JSON object keys must be strings).
    cards: HashMap<String, CardState>,
}

/// The progress store for all decks.
pub struct Store {
    path: PathBuf,
    cards: HashMap<u64, CardState>,
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
}

impl Store {
    /// Opens the store at `path`, creating an empty in-memory one if the file
    /// does not exist yet (it is written on the first [`save`](Self::save)).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();

        if !path.exists() {
            return Ok(Self { path, cards: HashMap::new() });
        }

        let text = std::fs::read_to_string(&path)
            .map_err(|source| StoreError::Io { path: path.clone(), source })?;
        let file: StoreFile = serde_json::from_str(&text)
            .map_err(|source| StoreError::Format { path: path.clone(), source })?;
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
        Ok(Self { path, cards })
    }

    /// Saves the store atomically (write to a temp file, then rename).
    pub fn save(&self) -> Result<(), StoreError> {
        let io_err = |source| StoreError::Io { path: self.path.clone(), source };

        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(io_err)?;
        }

        let file = StoreFile {
            version: 1,
            cards: self
                .cards
                .iter()
                .map(|(hash, state)| (hash.to_string(), state.clone()))
                .collect(),
        };
        let json = serde_json::to_string_pretty(&file)
            .map_err(|source| StoreError::Format { path: self.path.clone(), source })?;

        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(io_err)?;
        std::fs::rename(&tmp, &self.path).map_err(io_err)?;
        Ok(())
    }

    /// Returns the state of a card, if it has been seen before.
    pub fn get(&self, card_id: u64) -> Option<&CardState> {
        self.cards.get(&card_id)
    }

    /// Returns a mutable reference to the state of a card, inserting a fresh
    /// stage-1 state if the card is new.
    pub fn get_or_insert(&mut self, card_id: u64, now_ms: u64) -> &mut CardState {
        self.cards.entry(card_id).or_insert_with(|| CardState::new(now_ms))
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

/// The default location of the store file
/// (`~/.local/share/flash/progress.json` on Linux).
pub fn default_store_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "flash")
        .map(|dirs| dirs.data_dir().join("progress.json"))
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
        assert_eq!(vec![Review { ts_ms: 1000, passed: true }], state.history);
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
