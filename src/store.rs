//! The progress store.
//!
//! Progress is kept in a single JSON file (by default
//! `~/.local/share/alix/progress.json`), created on first save.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result as AnyResult, bail};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{deck, scheduler::Grade};

/// How many of the most recent reviews are kept per card.
const HISTORY_CAP: usize = 50;

/// The highest Leitner stage. Cards that keep passing stay here.
pub const MAX_STAGE: u8 = 5;

/// The on-disk store-format version. **Pinned at 1 pre-1.0** — per the project
/// convention we do not bump it or migrate: the shape changes freely and old data
/// is loaded best-effort via `#[serde(default)]` (surviving progress is a bonus, not
/// a guarantee). Versioning + migrations are a post-1.0 concern; the field is kept so
/// that door stays open.
const CURRENT_VERSION: u32 = 1;

/// One recorded review of a card.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Review {
    /// When the review happened (Unix ms).
    pub ts_ms: u64,
    /// The grade the card was answered with. Pre-grade stores logged only a pass/fail
    /// bool; those entries load with a default grade (a deliberate pre-1.0 break —
    /// scheduling state is unaffected).
    #[serde(default = "default_review_grade")]
    pub grade: Grade,
}

/// Serde default for a `Review` from a pre-grade store (which had only a `passed`
/// bool, no `grade`): assume a pass. History is cosmetic (stats + `last_review_ms`),
/// so a wrong default here cannot corrupt scheduling.
fn default_review_grade() -> Grade {
    Grade::Pass
}

/// FSRS memory state for a card — our own representation (all primitives + `u64`
/// times), kept decoupled from `rs-fsrs`'s `Card` so the store stays all-`u64` and
/// isn't tied to the crate's type. Present once the card has an FSRS review.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FsrsState {
    /// Days for retrievability to fall from 100% to 90%.
    pub stability: f64,
    /// Intrinsic difficulty (1..=10).
    pub difficulty: f64,
    /// Successful-review count (rs-fsrs bookkeeping).
    pub reps: u32,
    /// Lapse count.
    pub lapses: u32,
    /// rs-fsrs learning state (0 = New, 1 = Learning, 2 = Review, 3 = Relearning).
    pub state: u8,
    /// The interval this card was last scheduled for, in days — drives retirement.
    pub scheduled_days: u32,
    /// When the card was last reviewed (Unix ms).
    pub last_review_ms: u64,
    /// When the card is due next (Unix ms).
    pub due_ms: u64,
    /// Full `Good` grades accumulated in the initial acquisition phase (reset by a
    /// `Fail`; a `Partial` is neutral). Graduation to `Review` waits for two, so a
    /// fail can't fast-track a card past `Good → Good`. `serde(default)` so
    /// pre-existing stores read as 0 (no Goods yet).
    #[serde(default)]
    pub learning_goods: u8,
}

impl FsrsState {
    /// Whether the card has *graduated* the initial learning steps — reached FSRS
    /// `Review` (state 2) or beyond. A later lapse to `Relearning` (3) still counts;
    /// only `New`/`Learning` cards have not graduated.
    pub fn graduated(&self) -> bool {
        self.state >= 2
    }
}

/// The stored state of a single card, keyed by its identity hash.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CardState {
    /// Legacy Leitner stage (1..=5). No longer live scheduling state under FSRS — retained as an
    /// acquire marker and for the one-time lazy-derive that seeds FSRS from a pre-FSRS card's
    /// stage.
    pub stage: u8,
    /// When the card entered its current stage (Unix ms).
    pub stage_entered_ms: u64,
    /// FSRS state; present once the card has been reviewed under FSRS (or derived
    /// from the Leitner `stage` on its first FSRS review).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fsrs: Option<FsrsState>,
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
    /// The frontier depth rung this card is currently scheduled at. `fsrs`
    /// always represents *this* rung's schedule; a rung change resets it.
    #[serde(default)]
    pub rung: crate::ladder::Rung,
    /// Spaced passes accumulated since this rung graduated to FSRS `Review` —
    /// the climb signal (§3.3). Reset on any rung change and on a miss.
    #[serde(default)]
    pub passes_since_graduation: u8,
}

impl CardState {
    /// State for a card entering the system now at stage 1.
    pub fn new(now_ms: u64) -> Self {
        Self {
            stage: 1,
            stage_entered_ms: now_ms,
            fsrs: None,
            total_reviews: 0,
            total_passes: 0,
            streak: 0,
            history: Vec::new(),
            rung: crate::ladder::Rung::default(),
            passes_since_graduation: 0,
        }
    }

    /// Appends a review to the bounded history and updates the counters.
    pub fn record_review(&mut self, ts_ms: u64, grade: Grade) {
        self.total_reviews += 1;
        if grade.passed() {
            self.total_passes += 1;
            self.streak += 1;
        } else {
            self.streak = 0;
        }
        self.history.push(Review { ts_ms, grade });
        if self.history.len() > HISTORY_CAP {
            let excess = self.history.len() - HISTORY_CAP;
            self.history.drain(..excess);
        }
    }

    /// Moves the card to `rung`, starting a fresh schedule there: the higher
    /// rung is genuinely harder, so no stability is carried across (spec §3.6).
    pub fn set_rung(&mut self, rung: crate::ladder::Rung) {
        self.rung = rung;
        self.fsrs = None;
        self.passes_since_graduation = 0;
    }
}

/// Deck-level progress, keyed by deck subject (= file name): whether the deck's
/// AI exam has been passed ("mastered"), and when it was last *failed* (for the
/// re-sit cooldown). A deck appears here once either happens; an entry with
/// neither set is meaningless and is never written.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeckProgress {
    /// When the exam was last passed (Unix ms); `None` until it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mastered_at_ms: Option<u64>,
    /// When the exam was last failed (Unix ms), gating an immediate re-sit;
    /// `None` if it has never failed (or a later pass cleared it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exam_failed_at_ms: Option<u64>,
}

/// Which trigger produced a virtual card (see the virtual-cards spec, §2).
/// Only `Remediation` exists today; a future consumer (contrast, spin-off)
/// adds a variant here rather than growing a separate mechanism.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VirtualKind {
    /// Generated from a failed exam's gap, to drill the specific miss.
    Remediation,
}

/// A personally-scheduled card that lives in no deck file. Content is its
/// canonical one-card deck-format `text`; its schedule/history is a normal
/// `CardState` in `store.cards`, keyed by `id` (identical to a deck card).
/// Membership in `virtual_cards` is what makes a card "virtual".
///
/// Invariant: `id` == a `Card::id` in `parse(parent, text)` == the map key ==
/// the id its `CardState` is keyed under in `store.cards`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VirtualCard {
    /// The card's identity hash — a plain deck-card `Card::id` (via
    /// `parse(parent, text)`), also this entry's key in `virtual_cards`.
    pub id: u64,
    /// Which trigger produced this card.
    pub kind: VirtualKind,
    /// The deck subject (file name) this card belongs to. **Also the subject
    /// that `synthesize`/`promote` must parse/append under**, or the id won't
    /// reproduce (`Card::id` hashes the subject).
    pub parent: String,
    /// The card's canonical deck-format block (`# …`/`#? …` + its lines). For a
    /// cloze card this is the whole multi-hole `#?` block; the hole this entry
    /// stands for is identified by matching `id` against `parse(parent, text)`,
    /// not by a stored index.
    pub text: String,
    /// When this virtual card was created (Unix ms).
    pub created_ms: u64,
}

/// On-disk representation of the store.
#[derive(Serialize, Deserialize)]
struct StoreFile {
    /// Format version. Defaults to 1 for a file written before the field was
    /// required, so a legacy store still loads. Read but not gated on — see
    /// [`CURRENT_VERSION`] for why there is no version check pre-1.0.
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
    /// Virtual cards keyed by the decimal string of their `u64` id. Loaded
    /// **leniently** (see [`Store::open`]): the raw JSON value is kept so a
    /// stale/old-shape entry can be dropped without failing the whole file.
    /// Absent in a store with no virtual cards.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    virtual_cards: HashMap<String, serde_json::Value>,
}

/// The progress store for all decks.
pub struct Store {
    path: PathBuf,
    cards: HashMap<u64, CardState>,
    decks: HashMap<String, DeckProgress>,
    virtual_cards: HashMap<u64, VirtualCard>,
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
            return Ok(Self {
                path,
                cards: HashMap::new(),
                decks: HashMap::new(),
                virtual_cards: HashMap::new(),
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
        // The authored `cards` load stays strict: a bad card key is real
        // corruption, not a regenerable sidecar entry.
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
        // Virtual cards load leniently: a store from before this rework has
        // `v:`-prefixed keys and old-shape values, and a virtual card is a
        // personal, local, *regenerable* sidecar — so a stale entry is dropped
        // rather than failing the whole file (which would refuse a user's real
        // card progress). Keep only well-formed new-shape entries.
        let mut virtual_cards = HashMap::new();
        for (key, val) in file.virtual_cards {
            if let (Ok(id), Ok(vc)) = (
                key.parse::<u64>(),
                serde_json::from_value::<VirtualCard>(val),
            ) {
                virtual_cards.insert(id, vc);
            }
        }
        Ok(Self {
            path,
            cards,
            decks: file.decks,
            virtual_cards,
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

        // Emit the sidecar with decimal-`u64` keys and the real `VirtualCard`
        // shape (the lenient `Value` field is only for tolerant loading).
        let mut virtual_cards = HashMap::with_capacity(self.virtual_cards.len());
        for (id, vc) in &self.virtual_cards {
            let value = serde_json::to_value(vc).map_err(|source| StoreError::Format {
                path: self.path.clone(),
                source,
            })?;
            virtual_cards.insert(id.to_string(), value);
        }

        let file = StoreFile {
            version: CURRENT_VERSION,
            cards: self
                .cards
                .iter()
                .map(|(hash, state)| (hash.to_string(), state.clone()))
                .collect(),
            decks: self.decks.clone(),
            virtual_cards,
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

    /// Returns a virtual card by its `u64` id, if one exists.
    pub fn get_virtual(&self, id: u64) -> Option<&VirtualCard> {
        self.virtual_cards.get(&id)
    }

    /// Whether a card with this id is a virtual (remediation) card — i.e. it
    /// lives in the content sidecar rather than any deck file. This membership
    /// is the sole definition of "virtual"; its schedule is an ordinary
    /// `store.cards` entry.
    pub fn is_virtual(&self, id: u64) -> bool {
        self.virtual_cards.contains_key(&id)
    }

    /// Inserts or replaces a virtual card, keyed by its own `id`. The caller
    /// must uphold `card.id == its Card::id` (the map key). Does not save.
    pub fn insert_virtual(&mut self, card: VirtualCard) {
        self.virtual_cards.insert(card.id, card);
    }

    /// Drops a virtual card's content entry, e.g. once [`promote_virtual`] has
    /// graduated it into a real deck card. The card's schedule in `store.cards`
    /// (keyed by the same id) is left in place. Returns whether an entry was
    /// present. Does not save.
    pub fn remove_virtual(&mut self, id: u64) -> bool {
        self.virtual_cards.remove(&id).is_some()
    }

    /// Drops every sidecar `virtual_cards` entry that belongs to the same
    /// content block as `parent`/`text` — for a multi-hole cloze remediation
    /// card this is every hole's entry, not just one. Used by
    /// [`promote_virtual`] so promoting any one hole cleanly removes the whole
    /// block, leaving no orphaned sibling entries. Each entry's `store.cards`
    /// schedule is left in place. Returns how many entries were removed. Does
    /// not save.
    pub fn remove_virtual_block(&mut self, parent: &str, text: &str) -> usize {
        let before = self.virtual_cards.len();
        self.virtual_cards
            .retain(|_, vc| !(vc.parent == parent && vc.text == text));
        before - self.virtual_cards.len()
    }

    /// Every virtual card in the store, unfiltered — the raw building block
    /// behind [`virtual_cards_for`](Self::virtual_cards_for).
    pub fn iter_virtual_cards(&self) -> impl Iterator<Item = &VirtualCard> {
        self.virtual_cards.values()
    }

    /// Every virtual card belonging to deck `subject` (its `parent`), an exact
    /// match on the deck's file name. Includes derived-retired (archived)
    /// entries — callers filter those themselves for scheduling/counts (see
    /// [`crate::session::is_virtual_reviewable`]).
    pub fn virtual_cards_for(&self, subject: &str) -> Vec<&VirtualCard> {
        self.virtual_cards
            .values()
            .filter(|v| v.parent == subject)
            .collect()
    }

    /// Whether the given deck has passed its AI exam ("mastered").
    pub fn deck_mastered(&self, subject: &str) -> bool {
        self.deck_mastered_at(subject).is_some()
    }

    /// When the deck was mastered (epoch ms), if it has been.
    pub fn deck_mastered_at(&self, subject: &str) -> Option<u64> {
        self.decks.get(subject).and_then(|d| d.mastered_at_ms)
    }

    /// Records that the deck passed its exam at `now_ms`, clearing any failed-exam
    /// cooldown (a pass supersedes a prior fail). Does not save.
    pub fn set_deck_mastered(&mut self, subject: &str, now_ms: u64) {
        let entry = self.decks.entry(subject.to_string()).or_default();
        entry.mastered_at_ms = Some(now_ms);
        entry.exam_failed_at_ms = None;
    }

    /// When the deck's exam was last failed (epoch ms), if recently — drives the
    /// re-sit cooldown so a failed exam can't be immediately re-sat with the
    /// graded feedback pasted back in.
    pub fn exam_failed_at(&self, subject: &str) -> Option<u64> {
        self.decks.get(subject).and_then(|d| d.exam_failed_at_ms)
    }

    /// Records that the deck's exam was failed at `now_ms` (for the re-sit
    /// cooldown). Does not save.
    pub fn set_exam_failed(&mut self, subject: &str, now_ms: u64) {
        self.decks
            .entry(subject.to_string())
            .or_default()
            .exam_failed_at_ms = Some(now_ms);
    }

    /// Drops a deck's exam progress — both mastery and the failed-exam cooldown
    /// (e.g. on per-deck reset). Returns whether an entry was present. Does not
    /// save.
    pub fn clear_deck_mastered(&mut self, subject: &str) -> bool {
        self.decks.remove(subject).is_some()
    }

    /// Clears all stored progress, returning how many cards were removed (e.g.
    /// for `alix reset --all`). Also drops all deck-mastered state and every
    /// virtual card — a reset must not leave orphaned virtual cards behind to
    /// keep drilling. Does not save.
    pub fn clear(&mut self) -> usize {
        let n = self.cards.len();
        self.cards.clear();
        self.decks.clear();
        self.virtual_cards.clear();
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

    /// The number of virtual cards tracked by this store.
    pub fn virtual_len(&self) -> usize {
        self.virtual_cards.len()
    }

    /// The path of the store file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Graduates a virtual card into a real deck card: appends its stored
/// deck-format `text` to the deck file at `deck_path`, then drops the sidecar
/// content entry (or entries — see the cloze edge below) and saves the store.
///
/// The schedule needs no transfer: a virtual card's `CardState` already lives
/// in `store.cards` under the same id the appended deck card hashes to (the id
/// was unified at creation, not here), so the promoted card keeps its earned
/// schedule for free.
///
/// Appends **before** removing the sidecar entry: if the process dies between
/// the two steps, the card is merely duplicated (a sidecar entry plus a deck
/// card) rather than lost.
///
/// Cloze edge: a multi-hole `#?` block is stored as one sidecar entry per hole,
/// all sharing `parent` + the same whole-block `text`. Promoting one hole
/// appends the whole block, so the deck gains every hole as a real card — so
/// [`Store::remove_virtual_block`] drops every hole's sidecar entry, not just
/// the promoted one, leaving no orphans behind. Each hole's schedule carries
/// (its id matches its new deck sub-card).
pub fn promote_virtual(store: &mut Store, id: u64, deck_path: &Path) -> AnyResult<()> {
    let Some(vc) = store.get_virtual(id) else {
        bail!("no virtual card with id {id} to promote");
    };
    let text = vc.text.clone();
    let parent = vc.parent.clone();

    deck::append_cards(deck_path, &text)
        .with_context(|| format!("appending the promoted card to {}", deck_path.display()))?;

    // Drop the whole block's sidecar entries (all holes, for a cloze card);
    // the schedules stay in store.cards keyed by the same ids.
    store.remove_virtual_block(&parent, &text);
    store
        .save()
        .context("saving the store after promoting a virtual card")?;
    Ok(())
}

/// Serde default for a legacy store with no `version` field: the oldest format.
fn default_version() -> u32 {
    1
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
        store.get_or_insert(1, 0).record_review(100, Grade::Pass);
        store.get_or_insert(2, 0).record_review(300, Grade::Pass);
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
        state.record_review(1000, Grade::Pass);
        store.save().unwrap();

        let reloaded = Store::open(&path).unwrap();
        assert_eq!(1, reloaded.len());
        let state = reloaded.get(42).unwrap();
        assert_eq!(3, state.stage);
        assert_eq!(1, state.total_reviews);
        assert_eq!(
            vec![Review {
                ts_ms: 1000,
                grade: Grade::Pass
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
    fn exam_failed_records_and_a_pass_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");

        let mut store = Store::open(&path).unwrap();
        assert_eq!(None, store.exam_failed_at("t.txt"));
        // A failed exam stamps the cooldown without mastering the deck.
        store.set_exam_failed("t.txt", 5000);
        assert_eq!(Some(5000), store.exam_failed_at("t.txt"));
        assert!(!store.deck_mastered("t.txt"));
        store.save().unwrap();

        // Survives a save/reload.
        let mut reloaded = Store::open(&path).unwrap();
        assert_eq!(Some(5000), reloaded.exam_failed_at("t.txt"));
        // A later pass masters the deck and clears the cooldown.
        reloaded.set_deck_mastered("t.txt", 9000);
        assert!(reloaded.deck_mastered("t.txt"));
        assert_eq!(None, reloaded.exam_failed_at("t.txt"));
    }

    #[test]
    fn per_deck_clear_drops_the_cooldown_too() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.set_exam_failed("t.txt", 1);
        assert!(store.clear_deck_mastered("t.txt"));
        assert_eq!(None, store.exam_failed_at("t.txt"));
    }

    #[test]
    fn loads_a_v1_deck_record_with_a_bare_mastered_timestamp() {
        // Pre-v2 stores wrote `mastered_at_ms` as a bare number (not optional);
        // it must still load as a mastered deck.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(
            &path,
            "{\"version\":1,\"cards\":{},\"decks\":{\"rust.txt\":{\"mastered_at_ms\":1234}}}",
        )
        .unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.deck_mastered("rust.txt"));
        assert_eq!(Some(1234), store.deck_mastered_at("rust.txt"));
        assert_eq!(None, store.exam_failed_at("rust.txt"));
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
    fn loads_any_version_and_defaults_pre_grade_history() {
        // Pre-1.0 there is no version fence: any store loads best-effort. A store
        // whose history entries carried only a `passed` bool (no `grade`) still loads
        // — the scheduling state survives and the old entries get a default grade.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(
            &path,
            r#"{"version":999,"cards":{"5":{"stage":3,"stage_entered_ms":0,"history":[{"ts_ms":100,"passed":false}]}}}"#,
        )
        .unwrap();
        let store = Store::open(&path).unwrap();
        let state = store.get(5).unwrap();
        assert_eq!(3, state.stage); // scheduling state survives
        assert_eq!(100, state.history[0].ts_ms);
        assert_eq!(Grade::Pass, state.history[0].grade); // old `passed` dropped → default
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

    /// The canonical one-card deck-format `text` of a sample virtual card.
    const BORROW_TEXT: &str = "# What does the borrow checker enforce?\n\tExactly one mutable borrow, or many shared ones\n";

    /// Builds a virtual card from its canonical deck-format `text` under
    /// `parent`, deriving its id exactly as the substrate does — the `Card::id`
    /// of the (plain) card that `parse(parent, text)` yields. Seeds no schedule;
    /// a caller that needs the card scheduled adds a `store.cards` entry itself.
    fn virtual_card(parent: &str, text: &str) -> VirtualCard {
        let id = crate::parser::parse_str(parent, text).unwrap()[0].id();
        VirtualCard {
            id,
            kind: VirtualKind::Remediation,
            parent: parent.to_string(),
            text: text.to_string(),
            created_ms: 1000,
        }
    }

    #[test]
    fn insert_virtual_then_get_virtual_returns_it_with_fields_intact() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let vc = virtual_card("rust.txt", BORROW_TEXT);
        let id = vc.id;

        store.insert_virtual(vc);

        let got = store.get_virtual(id).unwrap();
        assert_eq!("rust.txt", got.parent);
        assert_eq!(VirtualKind::Remediation, got.kind);
        assert_eq!(BORROW_TEXT, got.text);
        assert!(store.is_virtual(id));
    }

    #[test]
    fn virtual_card_survives_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut store = Store::open(&path).unwrap();
        let vc = virtual_card("rust.txt", BORROW_TEXT);
        let id = vc.id;
        store.insert_virtual(vc.clone());
        store.save().unwrap();

        let reloaded = Store::open(&path).unwrap();
        let got = reloaded.get_virtual(id).unwrap();
        assert_eq!(&vc, got);
    }

    #[test]
    fn virtual_cards_for_matches_on_parent_subject() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        // Distinct ids come from distinct content (different back / subject).
        store.insert_virtual(virtual_card("rust.txt", "# f\n\tback one\n"));
        store.insert_virtual(virtual_card("rust.txt", "# f\n\tback two\n"));
        store.insert_virtual(virtual_card("other.txt", "# f\n\tback one\n"));

        let rust_cards = store.virtual_cards_for("rust.txt");
        assert_eq!(2, rust_cards.len());
        assert!(rust_cards.iter().all(|v| v.parent == "rust.txt"));

        assert_eq!(1, store.virtual_cards_for("other.txt").len());
        assert!(store.virtual_cards_for("nonexistent.txt").is_empty());
    }

    #[test]
    fn loads_store_file_without_virtual_cards_field() {
        // A store written before this field existed must still load — the
        // additive `#[serde(default)]` soft break, no version bump.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(&path, "{\"version\":1,\"cards\":{}}").unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.is_empty());
        assert!(store.get_virtual(123).is_none());
    }

    #[test]
    fn an_old_shape_virtual_cards_object_loads_leniently_dropping_stale_entries() {
        // A store from before this rework carries `v:`-prefixed keys AND old-shape
        // values (`state` + `content`, no `text`). Both make an entry unparseable
        // as the new `VirtualCard`; loading must drop them yet keep `cards`/`decks`
        // intact — refusing the whole file over a regenerable sidecar would lose a
        // user's real progress.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(
            &path,
            r#"{"version":1,
                "cards":{"5":{"stage":3,"stage_entered_ms":0}},
                "decks":{"rust.txt":{"mastered_at_ms":1234}},
                "virtual_cards":{
                    "v:abc":{"id":"v:abc","kind":"Remediation","parent":"rust.txt",
                             "content":{"front":"f","back":["b"],"mode":null},
                             "state":{"stage":1,"stage_entered_ms":0},"created_ms":0}
                }}"#,
        )
        .unwrap();
        let store = Store::open(&path).unwrap();
        // Real progress survives …
        assert_eq!(3, store.get(5).unwrap().stage);
        assert!(store.deck_mastered("rust.txt"));
        // … and the stale, regenerable virtual entry is dropped.
        assert_eq!(0, store.iter_virtual_cards().count());
    }

    fn write_deck(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn promote_virtual_appends_one_card_and_drops_the_virtual_entry() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = write_deck(dir.path(), "rust.txt", "# existing\n\tanswer\n");
        let store_path = dir.path().join("progress.json");
        let mut store = Store::open(&store_path).unwrap();
        let vc = virtual_card("rust.txt", BORROW_TEXT);
        let id = vc.id;
        store.insert_virtual(vc);

        promote_virtual(&mut store, id, &deck_path).unwrap();

        assert!(store.get_virtual(id).is_none());

        let text = std::fs::read_to_string(&deck_path).unwrap();
        let cards = crate::parser::parse_str("rust.txt", &text).unwrap();
        assert_eq!(2, cards.len());
        let promoted = cards
            .iter()
            .find(|c| c.front == "What does the borrow checker enforce?")
            .expect("promoted card present");
        assert_eq!(
            vec!["Exactly one mutable borrow, or many shared ones".to_string()],
            promoted.back
        );

        // The store was saved: reloading it from disk still shows the
        // virtual entry gone.
        let reloaded = Store::open(&store_path).unwrap();
        assert!(reloaded.get_virtual(id).is_none());
    }

    #[test]
    fn promote_leaves_existing_deck_card_ids_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = write_deck(dir.path(), "rust.txt", "# one\n\t1\n\n# two\n\t2\n");
        let before =
            crate::parser::parse_str("rust.txt", &std::fs::read_to_string(&deck_path).unwrap())
                .unwrap();
        let ids_before: Vec<u64> = before.iter().map(|c| c.id()).collect();

        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let vc = virtual_card("rust.txt", BORROW_TEXT);
        let id = vc.id;
        store.insert_virtual(vc);

        promote_virtual(&mut store, id, &deck_path).unwrap();

        let after =
            crate::parser::parse_str("rust.txt", &std::fs::read_to_string(&deck_path).unwrap())
                .unwrap();
        let ids_after: Vec<u64> = after.iter().take(2).map(|c| c.id()).collect();
        assert_eq!(ids_before, ids_after);
        assert_eq!(3, after.len()); // the two originals plus the promoted card
    }

    #[test]
    fn promote_unknown_id_errors_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = write_deck(dir.path(), "d.txt", "# one\n\t1\n");
        let deck_before = std::fs::read_to_string(&deck_path).unwrap();
        let store_path = dir.path().join("progress.json");
        let mut store = Store::open(&store_path).unwrap();

        let result = promote_virtual(&mut store, 999, &deck_path);

        assert!(result.is_err());
        assert_eq!(deck_before, std::fs::read_to_string(&deck_path).unwrap());
        assert!(!store_path.exists()); // save() was never reached
    }

    #[test]
    fn promoting_one_hole_of_a_multi_hole_cloze_removes_every_holes_sidecar_entry() {
        // A multi-hole cloze remediation card is stored as N sidecar entries (one
        // per hole), all sharing `parent` + the whole-block `text`, keyed by their
        // N distinct `Card::id`s. Promoting any one hole must clear the whole
        // block from the sidecar — not just the promoted hole's entry — or the
        // sibling holes become orphans whose ids collide with the now-real deck
        // cards (mis-counted, mis-badged, and a second promote would duplicate
        // ids in the deck file).
        let dir = tempfile::tempdir().unwrap();
        let deck_path = write_deck(dir.path(), "rust.txt", "# existing\n\tanswer\n");
        let store_path = dir.path().join("progress.json");
        let mut store = Store::open(&store_path).unwrap();

        let text = "#? Complete the quote\n\tTo {{be}} or not to {{be}}\n\t! Hamlet\n";
        let cards = crate::parser::parse_str("rust.txt", text).unwrap();
        assert_eq!(2, cards.len());
        let id0 = cards[0].id();
        let id1 = cards[1].id();
        assert_ne!(id0, id1, "the two holes must have distinct ids");

        for id in [id0, id1] {
            store.insert_virtual(VirtualCard {
                id,
                kind: VirtualKind::Remediation,
                parent: "rust.txt".to_string(),
                text: text.to_string(),
                created_ms: 1000,
            });
            // Seed a drilled schedule for each hole — promote must preserve these.
            store
                .get_or_insert(id, 1000)
                .record_review(1000, Grade::Pass);
        }

        promote_virtual(&mut store, id0, &deck_path).unwrap();

        // Both sidecar entries are gone — no orphan left for the sibling hole.
        assert!(store.get_virtual(id0).is_none());
        assert!(store.get_virtual(id1).is_none());
        // Both schedules survive: the promoted deck cards inherit their drilled
        // history for free.
        assert!(store.get(id0).is_some());
        assert!(store.get(id1).is_some());

        // The deck file gained the cloze card (both holes, since a cloze
        // promotes as one block).
        let deck_text = std::fs::read_to_string(&deck_path).unwrap();
        let deck_cards = crate::parser::parse_str("rust.txt", &deck_text).unwrap();
        assert_eq!(3, deck_cards.len()); // the existing plain card + 2 cloze holes

        // A second promote of the sibling hole now bails cleanly (its sidecar
        // entry is already gone) instead of re-appending the block and
        // duplicating ids in the deck file.
        let deck_before_second = std::fs::read_to_string(&deck_path).unwrap();
        let second = promote_virtual(&mut store, id1, &deck_path);
        assert!(second.is_err());
        assert_eq!(
            deck_before_second,
            std::fs::read_to_string(&deck_path).unwrap(),
            "a bailed second promote must not touch the deck file"
        );
    }

    #[test]
    fn promote_preserves_the_schedule_for_free() {
        // The schedule lives in `store.cards[id]` (not on the sidecar), keyed by
        // the same id the appended deck card hashes to — so promoting carries the
        // drilled schedule with no transfer code.
        let dir = tempfile::tempdir().unwrap();
        let deck_path = write_deck(dir.path(), "rust.txt", "# existing\n\tanswer\n");
        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let vc = virtual_card("rust.txt", BORROW_TEXT);
        let id = vc.id;
        store.insert_virtual(vc);

        // Drill the schedule in `store.cards`, not on the virtual entry.
        let mut state = CardState::new(1000);
        state.record_review(1000, Grade::Pass);
        state.record_review(2000, Grade::Pass);
        state.fsrs = Some(FsrsState {
            stability: 12.5,
            difficulty: 4.2,
            reps: 2,
            lapses: 0,
            state: 2,
            scheduled_days: 10,
            last_review_ms: 2000,
            due_ms: 900_000,
            learning_goods: 2,
        });
        *store.get_or_insert(id, 1000) = state.clone();

        promote_virtual(&mut store, id, &deck_path).unwrap();

        assert!(store.get_virtual(id).is_none());

        let text = std::fs::read_to_string(&deck_path).unwrap();
        let cards = crate::parser::parse_str("rust.txt", &text).unwrap();
        let promoted = cards
            .iter()
            .find(|c| c.front == "What does the borrow checker enforce?")
            .expect("promoted card present");
        // The id was unified at the source: the appended deck card hashes to the
        // very id the schedule was already keyed under.
        assert_eq!(id, promoted.id());
        let carried = store.get(promoted.id()).expect("schedule carried over");
        assert_eq!(&state, carried);
    }

    #[test]
    fn history_is_capped() {
        let mut state = CardState::new(0);
        for i in 0..(HISTORY_CAP as u64 + 10) {
            state.record_review(i, Grade::Pass);
        }
        assert_eq!(HISTORY_CAP, state.history.len());
        assert_eq!(10, state.history[0].ts_ms);
        assert_eq!(HISTORY_CAP as u32 + 10, state.total_reviews);
    }

    #[test]
    fn streak_resets_on_fail() {
        let mut state = CardState::new(0);
        state.record_review(1, Grade::Pass);
        state.record_review(2, Grade::Pass);
        assert_eq!(2, state.streak);
        state.record_review(3, Grade::Fail);
        assert_eq!(0, state.streak);
        assert_eq!(2, state.total_passes);
        assert_eq!(3, state.total_reviews);
    }

    #[test]
    fn record_review_stores_the_grade_and_partial_counts_as_a_pass() {
        let mut state = CardState::new(0);
        state.record_review(10, Grade::Partial);
        assert_eq!(Grade::Partial, state.history.last().unwrap().grade);
        assert_eq!(1, state.total_reviews);
        assert_eq!(1, state.total_passes); // Partial (a weak success) counts as a pass
        assert_eq!(1, state.streak);
    }

    #[test]
    fn set_rung_resets_the_fsrs_schedule_and_counter() {
        use crate::ladder::Rung;
        let mut s = CardState::new(0);
        s.rung = Rung::Recall;
        s.fsrs = Some(FsrsState {
            stability: 40.0,
            state: 2,
            scheduled_days: 30,
            ..Default::default()
        });
        s.passes_since_graduation = 3;

        s.set_rung(Rung::Reconstruct);

        assert_eq!(s.rung, Rung::Reconstruct);
        assert!(
            s.fsrs.is_none(),
            "a rung change starts a fresh schedule at the new rung"
        );
        assert_eq!(s.passes_since_graduation, 0);
    }

    #[test]
    fn a_pre_ladder_store_loads_cards_at_recall() {
        // Old stores have no `rung`/`passes_since_graduation` field.
        let json = r#"{"stage":1,"stage_entered_ms":0}"#;
        let state: CardState = serde_json::from_str(json).unwrap();
        assert_eq!(state.rung, crate::ladder::Rung::Recall);
        assert_eq!(state.passes_since_graduation, 0);
    }

    #[test]
    fn history_grades_survive_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut store = Store::open(&path).unwrap();
        let st = store.get_or_insert(7, 0);
        st.record_review(100, Grade::Partial);
        st.record_review(200, Grade::Fail);
        store.save().unwrap();

        let reloaded = Store::open(&path).unwrap();
        let history = &reloaded.get(7).unwrap().history;
        assert_eq!(Grade::Partial, history[0].grade);
        assert_eq!(Grade::Fail, history[1].grade);
    }

    #[test]
    fn fsrs_state_survives_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut store = Store::open(&path).unwrap();
        store.get_or_insert(9, 0).fsrs = Some(FsrsState {
            stability: 12.5,
            difficulty: 6.0,
            reps: 3,
            lapses: 1,
            state: 2,
            scheduled_days: 12,
            last_review_ms: 1000,
            due_ms: 2000,
            learning_goods: 1,
        });
        store.save().unwrap();
        let reloaded = Store::open(&path).unwrap();
        let f = reloaded.get(9).unwrap().fsrs.unwrap();
        assert_eq!(2000, f.due_ms);
        assert_eq!(1, f.learning_goods);
    }
}
