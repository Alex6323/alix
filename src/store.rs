//! The progress store.
//!
//! Progress is kept in a single JSON file (by default
//! `~/.local/share/alix/progress.json`), created on first save.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result as AnyResult, bail};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{card::Card, deck, depth::Depth, scheduler::Grade};

/// How many of the most recent reviews are kept per card.
const HISTORY_CAP: usize = 50;

/// The on-disk store-format version. **Pinned at 1 pre-1.0** — per the project
/// convention we do not bump it or migrate: the shape changes freely and old data
/// is loaded best-effort via `#[serde(default)]` (surviving progress is a bonus, not
/// a guarantee). Versioning + migrations are a post-1.0 concern; the field is kept so
/// that door stays open.
const CURRENT_VERSION: u32 = 1;

/// How recent a foreign write must be to warrant a warning: an older one is
/// ordinary roaming (yesterday's desktop session), not a likely concurrent
/// device. The rule lives here so every client warns the same way.
pub const FOREIGN_WRITE_WARN_WINDOW_MS: u64 = 60 * 60 * 1000;

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
    /// The depth this review was graded at. Pre-depth stores logged no depth at
    /// all (there was only ever one schedule) — those entries default to `Recall`.
    #[serde(default)]
    pub depth: Depth,
    /// Whether this review was credited downward from a pass at a higher depth
    /// (a full Reconstruct pass crediting a due Recall schedule) rather than
    /// answered directly. Directly-answered reviews don't serialize the field.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub propagated: bool,
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
    /// When the card was first acquired (Unix ms); the acquire-cooldown anchor for a
    /// not-yet-scheduled card.
    #[serde(default)]
    pub acquired_ms: u64,
    /// Recall-depth FSRS state; present once the card has been reviewed at
    /// Recall, absent for a not-yet-reviewed (or freshly acquired) card. Was
    /// `fsrs` — a clean pre-1.0 rename, no alias: a store carrying an old
    /// `fsrs` key simply loads this as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall: Option<FsrsState>,
    /// Reconstruct-depth FSRS state, independent of `recall` (stationarity: one
    /// schedule, one task, forever — no cross-crediting between depths).
    /// Lazily created on the card's first Reconstruct review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconstruct: Option<FsrsState>,
    /// When the card was first correctly picked at the Recognize depth (Unix
    /// ms); `None` until then. Recognize is unscheduled and boolean — this
    /// flag, not an `FsrsState`, is its only stored progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognized_ms: Option<u64>,
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
    /// State for a card entering the system now.
    pub fn new(now_ms: u64) -> Self {
        Self {
            acquired_ms: now_ms,
            recall: None,
            reconstruct: None,
            recognized_ms: None,
            total_reviews: 0,
            total_passes: 0,
            streak: 0,
            history: Vec::new(),
        }
    }

    /// The card's FSRS schedule at `depth`. `Recognize` is never scheduled
    /// (unscheduled + boolean) and always answers `None`.
    pub fn schedule(&self, depth: Depth) -> Option<&FsrsState> {
        match depth {
            Depth::Recognize => None,
            Depth::Recall => self.recall.as_ref(),
            Depth::Reconstruct => self.reconstruct.as_ref(),
        }
    }

    /// A mutable handle to the schedule slot at `depth`, for a scheduler to
    /// read/replace. `Recognize` has no slot to hand back.
    pub fn schedule_slot(&mut self, depth: Depth) -> Option<&mut Option<FsrsState>> {
        match depth {
            Depth::Recognize => None,
            Depth::Recall => Some(&mut self.recall),
            Depth::Reconstruct => Some(&mut self.reconstruct),
        }
    }

    /// Appends a review to the bounded history and updates the counters.
    /// `propagated` marks a review the learner never answered directly — credit
    /// that flowed down from a pass at a higher depth (see `Session::grade`).
    pub fn record_review(&mut self, ts_ms: u64, grade: Grade, depth: Depth, propagated: bool) {
        self.total_reviews += 1;
        if grade.passed() {
            self.total_passes += 1;
            self.streak += 1;
        } else {
            self.streak = 0;
        }
        self.history.push(Review {
            ts_ms,
            grade,
            depth,
            propagated,
        });
        if self.history.len() > HISTORY_CAP {
            let excess = self.history.len() - HISTORY_CAP;
            self.history.drain(..excess);
        }
    }
}

/// Deck-level progress, keyed by deck subject (= file name): whether the deck's
/// AI exam has been passed ("mastered"), when it was last *failed* (for the
/// re-sit cooldown), the learner's last-used session depth, and each badge's
/// first-earn date. A deck appears here once any of these is set; an entry
/// with nothing set is meaningless and is never written.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeckProgress {
    /// When the exam was last passed (Unix ms); `None` until it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mastered_at_ms: Option<u64>,
    /// When the exam was last failed (Unix ms), gating an immediate re-sit;
    /// `None` if it has never failed (or a later pass cleared it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exam_failed_at_ms: Option<u64>,
    /// The session depth the learner last chose for this deck — the
    /// plain-Learn button's memory across sessions. `None` until a depth has
    /// ever been recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_depth: Option<Depth>,
    /// When the Recognize badge was first earned (Unix ms); a high-water
    /// mark — see [`note_badges`]. `None` until earned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognized_at_ms: Option<u64>,
    /// When the Recall badge was first earned (Unix ms); see
    /// [`note_badges`]. `None` until earned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recalled_at_ms: Option<u64>,
    /// When the Reconstruct badge was first earned (Unix ms); see
    /// [`note_badges`]. `None` until earned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconstructed_at_ms: Option<u64>,
}

/// The community "mature" line (days of stability), in days — the badge
/// threshold for the Recall/Reconstruct FSRS schedules (`FsrsState::stability`
/// is already in days). See [`badge_solid`].
pub const MATURE_STABILITY_DAYS: f64 = 21.0;

/// Which trigger produced a virtual card (see the virtual-cards spec, §2).
/// Each variant represents a different source: generated from exam failures,
/// distilled from tutor exchanges, etc.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VirtualKind {
    /// Generated from a failed exam's gap, to drill the specific miss.
    Remediation,
    /// Distilled by the tutor from a review exchange (a "make this a card" action).
    Tutor,
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
    /// The card's identity token — a plain deck-card `Card::id` (via
    /// `parse(parent, text)`), also this entry's key in `virtual_cards`.
    pub id: String,
    /// Which trigger produced this card.
    pub kind: VirtualKind,
    /// The deck subject (file name) this card belongs to. **Also the subject
    /// that `synthesize`/`promote` must parse/append under**, so the block
    /// re-parses to the same sub-card ids (its content-identity fingerprint is
    /// computed under this parent).
    pub parent: String,
    /// The card's canonical stamped L1 block (`## …` + its lines). For a
    /// cloze card this is the whole multi-hole block; the
    /// hole this entry stands for is identified by matching `id` against
    /// `parse(parent, text)`, not by a stored index.
    pub text: String,
    /// When this virtual card was created (Unix ms).
    pub created_ms: u64,
}

/// The last-writer marker: which device wrote the store, and when. The
/// multi-device discipline is one device at a time; this marker turns a
/// silent violation into a visible warning (see
/// [`Store::recent_foreign_writer`]).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Writer {
    /// The writing device's label (see [`device_label`]).
    pub device: String,
    /// When that device last saved (Unix ms).
    pub at_ms: u64,
}

/// The fingerprint-records format version (spec §6). A one-byte tag on every
/// [`CardRecords`]: bumping it declares a store-data invalidation, so a change
/// to the §7 canonical form can't silently wipe hole schedules on unchanged
/// cards: stale-versioned records are ignored and rewritten, never mismatched.
/// Store-internal and freely changeable (never card identity).
pub const FP_VERSION: u8 = 1;

/// The two 64-bit fingerprints that let a cloze hole's schedule FOLLOW THE WORD
/// across edits (spec §3.4). Store-internal, non-frozen matcher data: `text_fp`
/// hashes the hole's hidden text, `line_fp` hashes its masked answer line
/// (both canonicalized in [`crate::l1`]). The matcher only tests equality, so
/// ~16 bytes/hole suffices (the raw strings are never stored).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HoleFingerprint {
    /// XxHash64 of the canonicalized hidden text.
    pub text_fp: u64,
    /// XxHash64 of the canonicalized answer line with this hole's span masked.
    pub line_fp: u64,
}

/// A card's realignment/reclaim records (spec §6), keyed in the store by the
/// card's base token. Store-internal, never identity, a serde-default soft add.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CardRecords {
    /// [`FP_VERSION`] at write time; a stale value is ignored and rewritten.
    pub version: u8,
    /// The block-level §7 content fingerprint
    /// ([`crate::l1::content_fingerprint`]): powers the §1.7 lost-comment
    /// reclaim and the precise doctor tell.
    pub content_fp: u64,
    /// Each hole's fingerprint in recorded (document) order; empty for a plain
    /// card. The cascade compares the file's current holes against these.
    pub holes: Vec<HoleFingerprint>,
}

/// A hole schedule evicted from a live `token-N` key when its word vanished
/// from a card (spec §3.4): the reclaim shelf. Never left squatting on a live
/// key where a new word could inherit the old word's schedule; un-pruned and
/// reclaim-eligible per §5, cleared only by an explicit reset.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OrphanedHole {
    /// The base token the evicted hole belonged to.
    pub token: String,
    /// The evicted hole's fingerprint (the matcher key if it ever returns).
    pub fp: HoleFingerprint,
    /// The schedule/history the hole carried when it was evicted.
    pub state: CardState,
}

/// The outcome of realigning one card's holes ([`realign_holes`]): a pure
/// function over fingerprints, so it speaks in hole indices; the store
/// application composes the full `token-N` ids from the card's token.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CascadeOutcome {
    /// Stored hole index → new file hole index for each matched hole. The
    /// schedule under `token-<old>` moves to `token-<new>`.
    pub remap: Vec<(u32, u32)>,
    /// Stored hole indices with no match: their schedules orphan to the shelf.
    pub orphaned: Vec<u32>,
    /// File hole indices with no history: fresh holes, no schedule yet.
    pub fresh: Vec<u32>,
}

/// Re-attach stored hole schedules to a card's current file holes so a hole's
/// progress FOLLOWS THE WORD, not the position (spec §3.4). Two passes, each
/// walking file holes in document order and consuming the FIRST unconsumed
/// stored record (in recorded order) that matches:
///
/// 1. exact `(text_fp, line_fp)` pairs anchor first (context intact);
/// 2. leftovers match by `text_fp` alone (a rewritten context, or an identical-word twin).
///
/// Leftover stored records orphan; leftover file holes are fresh. The
/// document-order tie-break makes identical twins pair index-for-index (a swap
/// is observationally a no-op), and keeps the result deterministic.
pub fn realign_holes(stored: &[HoleFingerprint], file: &[HoleFingerprint]) -> CascadeOutcome {
    let mut consumed = vec![false; stored.len()];
    // For each file hole, the stored record it matched (if any).
    let mut matched: Vec<Option<usize>> = vec![None; file.len()];

    // Pass 1: exact text + context.
    for (fi, fh) in file.iter().enumerate() {
        for (si, sh) in stored.iter().enumerate() {
            if !consumed[si] && sh.text_fp == fh.text_fp && sh.line_fp == fh.line_fp {
                consumed[si] = true;
                matched[fi] = Some(si);
                break;
            }
        }
    }
    // Pass 2: text alone, for the file holes pass 1 left unmatched.
    for (fi, fh) in file.iter().enumerate() {
        if matched[fi].is_some() {
            continue;
        }
        for (si, sh) in stored.iter().enumerate() {
            if !consumed[si] && sh.text_fp == fh.text_fp {
                consumed[si] = true;
                matched[fi] = Some(si);
                break;
            }
        }
    }

    let mut remap = Vec::new();
    let mut fresh = Vec::new();
    for (fi, m) in matched.iter().enumerate() {
        match m {
            Some(si) => remap.push((*si as u32, fi as u32)),
            None => fresh.push(fi as u32),
        }
    }
    remap.sort_unstable();
    let orphaned = (0..stored.len())
        .filter(|si| !consumed[*si])
        .map(|si| si as u32)
        .collect();
    CascadeOutcome {
        remap,
        orphaned,
        fresh,
    }
}

/// On-disk representation of the store.
#[derive(Serialize, Deserialize)]
struct StoreFile {
    /// Format version. Defaults to 1 for a file written before the field was
    /// required, so a legacy store still loads. Read but not gated on — see
    /// [`CURRENT_VERSION`] for why there is no version check pre-1.0.
    #[serde(default = "default_version")]
    version: u32,
    /// Card states keyed by the card's identity token (its `Card::id`).
    cards: HashMap<String, CardState>,
    /// Per-card realignment/reclaim records keyed by base token (spec §6). A
    /// serde-default soft add: a store from before this field loads with none.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    records: HashMap<String, CardRecords>,
    /// The reclaim shelf: hole schedules evicted from live keys (spec §3.4).
    /// A serde-default soft add.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hole_orphans: Vec<OrphanedHole>,
    /// Deck-level progress keyed by subject. Optional: a store written before
    /// this field existed (or with no mastered decks) simply has no `decks`
    /// key.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    decks: HashMap<String, DeckProgress>,
    /// Virtual cards keyed by their identity token. Loaded **leniently** (see
    /// [`Store::open`]): the raw JSON value is kept so a stale/old-shape entry
    /// can be dropped without failing the whole file. Absent in a store with no
    /// virtual cards.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    virtual_cards: HashMap<String, serde_json::Value>,
    /// Who wrote this file last. Absent in stores from before the field or
    /// written by an unnamed consumer; loaded leniently via the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    writer: Option<Writer>,
}

/// The progress store for all decks.
pub struct Store {
    path: PathBuf,
    cards: HashMap<String, CardState>,
    decks: HashMap<String, DeckProgress>,
    virtual_cards: HashMap<String, VirtualCard>,
    /// Per-card realignment/reclaim records, keyed by base token (spec §6).
    records: HashMap<String, CardRecords>,
    /// The reclaim shelf: hole schedules evicted from live keys (spec §3.4).
    hole_orphans: Vec<OrphanedHole>,
    /// This device's label, stamped into the file on every save. `None` (the
    /// default) leaves the file's existing marker untouched, so tests and
    /// one-off tools do not masquerade as a device.
    pub device: Option<String>,
    /// The marker loaded from disk: the device that wrote this store last.
    last_writer: Option<Writer>,
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
                records: HashMap::new(),
                hole_orphans: Vec::new(),
                device: None,
                last_writer: None,
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
        // Card keys are identity tokens (strings): they pass through verbatim.
        // A key of an unexpected charset is kept rather than rejected — it
        // becomes doctor material, not a load failure that would refuse a user's
        // real progress.
        let cards = file.cards;
        // Virtual cards load leniently: a store from before this rework has
        // old-shape values (a numeric `id`), and a virtual card is a personal,
        // local, *regenerable* sidecar — so a stale entry is dropped rather than
        // failing the whole file (which would refuse a user's real card
        // progress). Keep only well-formed new-shape entries.
        let mut virtual_cards = HashMap::new();
        for (key, val) in file.virtual_cards {
            if let Ok(vc) = serde_json::from_value::<VirtualCard>(val) {
                virtual_cards.insert(key, vc);
            }
        }
        Ok(Self {
            path,
            cards,
            decks: file.decks,
            virtual_cards,
            records: file.records,
            hole_orphans: file.hole_orphans,
            device: None,
            last_writer: file.writer,
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

        // Emit the sidecar with token keys and the real `VirtualCard` shape (the
        // lenient `Value` field is only for tolerant loading).
        let mut virtual_cards = HashMap::with_capacity(self.virtual_cards.len());
        for (id, vc) in &self.virtual_cards {
            let value = serde_json::to_value(vc).map_err(|source| StoreError::Format {
                path: self.path.clone(),
                source,
            })?;
            virtual_cards.insert(id.clone(), value);
        }

        let file = StoreFile {
            version: CURRENT_VERSION,
            cards: self.cards.clone(),
            records: self.records.clone(),
            hole_orphans: self.hole_orphans.clone(),
            decks: self.decks.clone(),
            virtual_cards,
            // An unnamed consumer preserves the marker on disk rather than
            // erasing what the last real device wrote.
            writer: self
                .device
                .clone()
                .map(|device| Writer {
                    device,
                    at_ms: crate::time::now_ms(),
                })
                .or_else(|| self.last_writer.clone()),
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

    /// The device that last wrote this store when it was not `my_device`:
    /// `(device, age_ms)` relative to `now_ms`. `None` when the store is
    /// unmarked or this device wrote it itself.
    pub fn foreign_writer(&self, my_device: &str, now_ms: u64) -> Option<(String, u64)> {
        let writer = self.last_writer.as_ref()?;
        if writer.device == my_device {
            return None;
        }
        Some((writer.device.clone(), now_ms.saturating_sub(writer.at_ms)))
    }

    /// [`foreign_writer`](Self::foreign_writer) filtered to
    /// [`FOREIGN_WRITE_WARN_WINDOW_MS`]: the "another device wrote this
    /// moments ago" case a client surfaces before the user reviews on top
    /// of a likely fork.
    pub fn recent_foreign_writer(&self, my_device: &str, now_ms: u64) -> Option<(String, u64)> {
        self.foreign_writer(my_device, now_ms)
            .filter(|(_, age_ms)| *age_ms < FOREIGN_WRITE_WARN_WINDOW_MS)
    }

    /// Returns the state of a card, if it has been seen before.
    pub fn get(&self, card_id: &str) -> Option<&CardState> {
        self.cards.get(card_id)
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

    /// Returns a mutable reference to the state of a card, inserting a freshly
    /// acquired state (no FSRS schedule yet) if the card is new.
    pub fn get_or_insert(&mut self, card_id: &str, now_ms: u64) -> &mut CardState {
        self.cards
            .entry(card_id.to_string())
            .or_insert_with(|| CardState::new(now_ms))
    }

    /// Drops a card's stored state, e.g. when the card is deleted from its
    /// deck. Returns whether an entry was present. Does not save.
    pub fn remove(&mut self, card_id: &str) -> bool {
        self.cards.remove(card_id).is_some()
    }

    /// The realignment/reclaim records for a base token, if any (spec §6).
    pub fn records(&self, token: &str) -> Option<&CardRecords> {
        self.records.get(token)
    }

    /// The reclaim shelf: hole schedules evicted from live keys (spec §3.4).
    pub fn hole_orphans(&self) -> &[OrphanedHole] {
        &self.hole_orphans
    }

    /// Content-fingerprint → orphaned token, for the §1.7 lost-comment reclaim:
    /// for each `wanted` content fingerprint, an ORPHANED base token (not in
    /// `live`, but still carrying a stored schedule) whose records hold that
    /// fingerprint: the token an unstamped card of that content re-adopts. At
    /// most one token per fingerprint (lowest token breaks a tie, for
    /// determinism); empty when nothing matches. Read-only.
    pub fn reclaim_candidates(
        &self,
        live: &HashSet<String>,
        wanted: &HashSet<u64>,
    ) -> HashMap<u64, String> {
        use std::collections::hash_map::Entry;
        let mut best: HashMap<u64, String> = HashMap::new();
        for (token, rec) in &self.records {
            if !wanted.contains(&rec.content_fp) || live.contains(token) {
                continue;
            }
            // A genuine orphan still has a schedule to preserve: under the base
            // token (plain) or a `token-N` sub-key (cloze). The `-` delimiter is
            // outside the token alphabet, so the prefix test can't cross tokens.
            let prefix = format!("{token}-");
            let has_schedule = self
                .cards
                .keys()
                .any(|key| key == token || key.starts_with(&prefix));
            if !has_schedule {
                continue;
            }
            match best.entry(rec.content_fp) {
                Entry::Vacant(v) => {
                    v.insert(token.clone());
                }
                Entry::Occupied(mut o) => {
                    if token < o.get() {
                        o.insert(token.clone());
                    }
                }
            }
        }
        best
    }

    /// Write (or refresh) a card's [`CardRecords`] under its base token: the
    /// §6 invariant that no store entry exists without its records. Called at
    /// every entry-creation site (review-open, the tutor/remediation mints, the
    /// trace-walk grade). A no-op for an unstamped card (no token to key on).
    /// Does NOT run the hole cascade (that is [`realign_card_holes`], which
    /// reads the OLD records before this overwrites them). Does not save.
    pub fn ensure_records(&mut self, card: &Card) {
        if let Some(token) = card.token.as_deref() {
            self.ensure_records_raw(token, card.content_fingerprint, &card.block_holes);
        }
    }

    /// [`ensure_records`] for a caller that has an id + fingerprints but no
    /// `Card` (the trace-walk grade path, whose checkpoints carry only an id).
    /// `token` is the base token; a plain card's id already is its token.
    pub fn ensure_records_raw(&mut self, token: &str, content_fp: u64, holes: &[HoleFingerprint]) {
        self.records.insert(
            token.to_string(),
            CardRecords {
                version: FP_VERSION,
                content_fp,
                holes: holes.to_vec(),
            },
        );
    }

    /// Re-attach a cloze card's stored hole schedules to its current file holes
    /// (spec §3.4), then refresh its records to the current version + file
    /// holes. Returns the [`CascadeOutcome`] when a realignment actually MOVED
    /// schedules (the card had current-version records whose holes disagreed
    /// with the file) so the caller can move the matching augment entries; `None`
    /// when nothing moved: no prior records, a stale [`FP_VERSION`] (ignored and
    /// rewritten, never mismatched: a canon tweak must not wipe schedules on
    /// unchanged cards), or the holes already agree. Does not save.
    ///
    /// The cascade REBUILDS this token's `token-N` entries into a fresh map:
    /// fingerprinted moves win contested keys, a fresh hole wins its live key
    /// (it simply has no entry until reviewed), and unmatched (orphaned) records
    /// are evicted to the shelf.
    pub fn realign_card_holes(
        &mut self,
        token: &str,
        file_holes: &[HoleFingerprint],
        content_fp: u64,
    ) -> Option<CascadeOutcome> {
        let outcome = match self.records.get(token) {
            // Current-version records whose holes disagree: run the cascade.
            Some(rec) if rec.version == FP_VERSION && rec.holes != file_holes => {
                let stored = rec.holes.clone();
                let outcome = realign_holes(&stored, file_holes);
                self.apply_hole_cascade(token, &stored, &outcome);
                Some(outcome)
            }
            // No records, a stale version, or holes already agree: no cascade.
            // Schedules stay positionally as they are; records are refreshed
            // below to the current version + file holes.
            _ => None,
        };
        self.ensure_records_raw(token, content_fp, file_holes);
        outcome
    }

    /// Rebuild `token`'s `token-N` schedule entries per a [`CascadeOutcome`]:
    /// matched holes move to their new index, and every unmapped survivor (a
    /// named orphan or a stray out-of-range entry) evicts to the shelf; fresh
    /// indices are left with no entry. `stored_holes` are the pre-cascade records
    /// (for the evicted fingerprints). Does not save.
    fn apply_hole_cascade(
        &mut self,
        token: &str,
        stored_holes: &[HoleFingerprint],
        outcome: &CascadeOutcome,
    ) {
        // Pull EVERY `token-N` hole entry into a fresh map keyed by old hole
        // index, not just `0..stored_holes.len()`: a stray entry above the range
        // must be pulled too, so it can't survive to be inherited by a future
        // fresh hole.
        let prefix = format!("{token}-");
        let hole_keys: Vec<(u32, String)> = self
            .cards
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .filter_map(|key| match crate::token::parse_card_id(key) {
                Some((_, Some(n), false)) => Some((n, key.clone())),
                _ => None,
            })
            .collect();
        let mut old: HashMap<u32, CardState> = HashMap::new();
        for (n, key) in hole_keys {
            if let Some(state) = self.cards.remove(&key) {
                old.insert(n, state);
            }
        }
        for (from, to) in &outcome.remap {
            if let Some(state) = old.remove(from) {
                let key = crate::token::card_id(token, Some(*to), false);
                self.cards.insert(key, state);
            }
        }
        // Every unmapped survivor evicts to the shelf: the cascade's named
        // orphans and any stray high-index entry alike. An in-range index carries
        // its stored fingerprint; a stray (no record) carries the default.
        let mut leftover: Vec<u32> = old.keys().copied().collect();
        leftover.sort_unstable();
        for orphan in leftover {
            if let Some(state) = old.remove(&orphan) {
                let fp = stored_holes
                    .get(orphan as usize)
                    .copied()
                    .unwrap_or_default();
                self.hole_orphans.push(OrphanedHole {
                    token: token.to_string(),
                    fp,
                    state,
                });
            }
        }
    }

    /// Returns a virtual card by its id, if one exists.
    pub fn get_virtual(&self, id: &str) -> Option<&VirtualCard> {
        self.virtual_cards.get(id)
    }

    /// Whether this id is a virtual card (remediation or tutor-minted), i.e. it
    /// lives in the content sidecar rather than any deck file. This membership
    /// is the sole definition of "virtual"; its schedule is an ordinary
    /// `store.cards` entry.
    pub fn is_virtual(&self, id: &str) -> bool {
        self.virtual_cards.contains_key(id)
    }

    /// Inserts or replaces a virtual card, keyed by its own `id`. The caller
    /// must uphold `card.id == its Card::id` (the map key). Does not save.
    pub fn insert_virtual(&mut self, card: VirtualCard) {
        self.virtual_cards.insert(card.id.clone(), card);
    }

    /// Drops a virtual card's content entry, e.g. once [`promote_virtual`] has
    /// graduated it into a real deck card. The card's schedule in `store.cards`
    /// (keyed by the same id) is left in place. Returns whether an entry was
    /// present. Does not save.
    pub fn remove_virtual(&mut self, id: &str) -> bool {
        self.virtual_cards.remove(id).is_some()
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

    /// The ids of the virtual cards on `subject` whose canonical content (§7)
    /// matches `fingerprint`. Empty when none do. This is the content-identity
    /// dedup key for remediation/tutor mint: a freshly minted card carries a new
    /// random token, so it can never dedup by id against an earlier mint of the
    /// same content, only by fingerprint. A multi-hole cloze block's holes all
    /// share the block fingerprint, so this returns every hole's id for a matched
    /// block, letting the caller revive or dedup the block as a unit.
    pub fn virtual_ids_with_content(&self, subject: &str, fingerprint: u64) -> Vec<String> {
        self.virtual_cards
            .values()
            .filter(|vc| vc.parent == subject && virtual_fingerprint(vc) == Some(fingerprint))
            .map(|vc| vc.id.clone())
            .collect()
    }

    /// Every virtual card belonging to deck `subject` (its `parent`), an exact
    /// match on the deck's file name. Includes derived-retired (archived)
    /// entries: callers filter those themselves for scheduling/counts (see
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

    /// The session depth the learner last used for `subject` — the
    /// plain-Learn button's memory across sessions. `None` until a depth has
    /// ever been recorded for this deck.
    pub fn last_depth(&self, subject: &str) -> Option<Depth> {
        self.decks.get(subject).and_then(|d| d.last_depth)
    }

    /// Records `depth` as the last-used session depth for `subject`. Does not
    /// save.
    pub fn set_last_depth(&mut self, subject: &str, depth: Depth) {
        self.decks
            .entry(subject.to_string())
            .or_default()
            .last_depth = Some(depth);
    }

    /// When the badge at `depth` was first earned for `subject` (Unix ms), if
    /// ever — the high-water mark [`note_badges`] maintains. `None` if it has
    /// never been earned.
    pub fn badge_earned(&self, subject: &str, depth: Depth) -> Option<u64> {
        let deck = self.decks.get(subject)?;
        match depth {
            Depth::Recognize => deck.recognized_at_ms,
            Depth::Recall => deck.recalled_at_ms,
            Depth::Reconstruct => deck.reconstructed_at_ms,
        }
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
        self.records.clear();
        self.hole_orphans.clear();
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

    /// The store keys with no matching id in the live workspace (spec §5): the
    /// orphaned-key check. A card-family key is an orphan when no enumerated
    /// deck card claims it AND it is not a virtual card's own schedule (those
    /// are legitimate local cards with no deck file). A deck-family key
    /// (subject) is an orphan when no enumerated deck carries that file name.
    /// Both are sorted for a stable report. Orphans are evidence (a stripped
    /// comment, a hand-deleted deck, a same-machine double-mint) and the reclaim
    /// pool. This never removes them; [`prune_orphans`](Self::prune_orphans)
    /// does, only under an explicit `alix reset --orphans`.
    pub fn orphans(
        &self,
        known_card_ids: &HashSet<String>,
        known_subjects: &HashSet<String>,
    ) -> Orphans {
        let mut cards: Vec<String> = self
            .cards
            .keys()
            .filter(|k| !known_card_ids.contains(*k) && !self.virtual_cards.contains_key(*k))
            .cloned()
            .collect();
        let mut decks: Vec<String> = self
            .decks
            .keys()
            .filter(|k| !known_subjects.contains(*k))
            .cloned()
            .collect();
        cards.sort();
        decks.sort();
        Orphans { cards, decks }
    }

    /// Removes exactly the orphaned keys in `orphans` (the `alix reset
    /// --orphans` action), returning how many keys were dropped. Does not save.
    pub fn prune_orphans(&mut self, orphans: &Orphans) -> usize {
        let mut removed = 0;
        for id in &orphans.cards {
            if self.cards.remove(id).is_some() {
                removed += 1;
            }
        }
        for subject in &orphans.decks {
            if self.decks.remove(subject).is_some() {
                removed += 1;
            }
        }
        // Once a pruned token has no schedule left, its records and shelf entries
        // can never be reclaimed (reclaim needs a schedule to preserve), so they
        // are dead weight and go with it. They are not counted: they are internal
        // bookkeeping, not keys the user was shown. A shelf entry whose token is
        // still LIVE stays (evidence for a future per-hole reclaim, bounded by
        // real edits), not garbage.
        let pruned_tokens: HashSet<&str> = orphans
            .cards
            .iter()
            .filter_map(|id| crate::token::parse_card_id(id).map(|(token, _, _)| token))
            .collect();
        for token in pruned_tokens {
            let prefix = format!("{token}-");
            let still_scheduled = self
                .cards
                .keys()
                .any(|key| key == token || key.starts_with(&prefix));
            if still_scheduled {
                continue;
            }
            self.records.remove(token);
            self.hole_orphans.retain(|orphan| orphan.token != token);
        }
        removed
    }

    /// Wipes a replaced deck's store state (spec §7): every authored card
    /// schedule whose base token is in `tokens`, those tokens' realignment
    /// records and reclaim-shelf entries, the deck-family entry keyed by
    /// `subject`, and every virtual card parented to `subject` (with its own
    /// schedule). Returns how many authored card schedules were removed.
    /// Deliberate wholesale destruction: nothing it touches is left half-removed
    /// to orphan later. Does not save.
    pub fn wipe_deck(&mut self, tokens: &HashSet<String>, subject: &str) -> usize {
        let doomed: Vec<String> = self
            .cards
            .keys()
            .filter(|id| {
                crate::token::parse_card_id(id).is_some_and(|(token, _, _)| tokens.contains(token))
            })
            .cloned()
            .collect();
        let mut wiped = 0;
        for id in doomed {
            if self.cards.remove(&id).is_some() {
                wiped += 1;
            }
        }
        for token in tokens {
            self.records.remove(token);
        }
        self.hole_orphans
            .retain(|orphan| !tokens.contains(&orphan.token));
        self.decks.remove(subject);
        let virtuals: Vec<String> = self
            .virtual_cards
            .values()
            .filter(|vc| vc.parent == subject)
            .map(|vc| vc.id.clone())
            .collect();
        for id in virtuals {
            self.virtual_cards.remove(&id);
            self.cards.remove(&id);
        }
        wiped
    }
}

/// The store keys matching no known card/deck in the scanned workspace (spec
/// §5): never auto-pruned (they are evidence and the reclaim pool), cleared
/// only by an explicit `alix reset --orphans`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Orphans {
    /// Orphaned card-family keys (card ids), sorted.
    pub cards: Vec<String>,
    /// Orphaned deck-family keys (subjects), sorted.
    pub decks: Vec<String>,
}

impl Orphans {
    /// Whether there are no orphaned keys at all.
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty() && self.decks.is_empty()
    }

    /// Total orphaned keys across both families.
    pub fn len(&self) -> usize {
        self.cards.len() + self.decks.len()
    }
}

/// Milliseconds left on a failed trace exam's re-sit cooldown, or `None` if it
/// can be sat now: it never failed, the cooldown has elapsed, or the cooldown
/// is disabled (`cooldown_secs == 0`). The launch sites (the web `Take exam`
/// among them) gate on this so the graded feedback can't be pasted straight
/// back into the one fixed trace question.
pub fn cooldown_remaining_ms(
    store: &Store,
    subject: &str,
    cooldown_secs: u64,
    now_ms: u64,
) -> Option<u64> {
    if cooldown_secs == 0 {
        return None;
    }
    let until = store
        .exam_failed_at(subject)?
        .saturating_add(cooldown_secs.saturating_mul(1000));
    (until > now_ms).then(|| until - now_ms)
}

/// Why a tutor-minted card could not be added.
#[derive(Debug, thiserror::Error)]
pub enum MintError {
    /// The front/back did not form exactly one well-formed card.
    #[error("the drafted card is malformed: {0}")]
    Malformed(String),
    /// A card with this content already exists in the deck.
    #[error("a card with this content already exists in the deck")]
    Duplicate,
    /// The OS CSPRNG failed while minting the identity token.
    #[error("cannot mint an identity token: {0}")]
    Mint(String),
}

/// Mints a free-standing `Tutor` virtual card on `subject` from an edited
/// front/back: builds the L1 card block with a freshly minted identity token,
/// parses it under `subject` for its id, rejects a malformed block, then
/// inserts it and seeds a fresh schedule so it enters the queue as a new
/// (acquire) card. Returns the new id.
///
/// `deck_fingerprints` are the §7 canonical-content fingerprints of the deck's
/// authored cards ([`crate::l1::content_fingerprint`]). The `Duplicate` check
/// is by content, not id: every mint carries a fresh random token, so identical
/// content would otherwise mint a distinct card. A card whose content already
/// exists in the deck or in a virtual card on this subject is rejected.
pub fn mint_tutor_card(
    store: &mut Store,
    subject: &str,
    front: &str,
    back: &[String],
    now_ms: u64,
    deck_fingerprints: &std::collections::HashSet<u64>,
) -> Result<String, MintError> {
    let front = front.trim();
    let back: Vec<String> = back
        .iter()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if front.is_empty() || back.is_empty() {
        return Err(MintError::Malformed(
            "front and back must both be non-empty".to_string(),
        ));
    }
    if front.contains('\n') || back.iter().any(|l| l.contains('\n')) {
        return Err(MintError::Malformed(
            "front and back must be single lines".to_string(),
        ));
    }
    let token = crate::token::mint().map_err(|e| MintError::Mint(e.to_string()))?;
    let mut text = format!("## {front} <!-- id: {token} -->\n");
    for line in &back {
        text.push_str(line);
        text.push('\n');
    }
    let cards =
        crate::l1::parse_str(subject, &text).map_err(|e| MintError::Malformed(e.to_string()))?;
    let [card] = cards.as_slice() else {
        return Err(MintError::Malformed(
            "expected exactly one card".to_string(),
        ));
    };
    let id = card
        .id()
        .ok_or_else(|| MintError::Malformed("the minted card has no identity token".to_string()))?;
    // Canonical-content dedup: reject if this exact content is already drillable
    // (a deck card) or already stored as a virtual card on this subject.
    let fingerprint = card.content_fingerprint;
    if deck_fingerprints.contains(&fingerprint)
        || !store
            .virtual_ids_with_content(subject, fingerprint)
            .is_empty()
    {
        return Err(MintError::Duplicate);
    }
    store.insert_virtual(VirtualCard {
        id: id.clone(),
        kind: VirtualKind::Tutor,
        parent: subject.to_string(),
        text,
        created_ms: now_ms,
    });
    // Invariant (§6): records exist before the schedule entry does.
    store.ensure_records(card);
    store.get_or_insert(&id, now_ms);
    Ok(id)
}

/// Live badge check: whether every card in `cards` is currently solid at
/// `depth` — Recognize needs every card's `recognized_ms` set; Recall and
/// Reconstruct need every card's `schedule(depth)` present with stability at
/// or past the mature line ([`MATURE_STABILITY_DAYS`]). An empty deck is
/// never solid. Pure — this only answers the live question; earning a badge
/// (persisting its first-earn date) is [`note_badges`].
pub fn badge_solid(cards: &[Card], store: &Store, depth: Depth) -> bool {
    if cards.is_empty() {
        return false;
    }
    cards.iter().all(|card| {
        let Some(state) = card.id().and_then(|id| store.get(&id)) else {
            return false;
        };
        match depth {
            Depth::Recognize => state.recognized_ms.is_some(),
            Depth::Recall | Depth::Reconstruct => state
                .schedule(depth)
                .is_some_and(|fsrs| fsrs.stability >= MATURE_STABILITY_DAYS),
        }
    })
}

/// Persists the first-earn date for any depth of `subject` that is currently
/// solid, per [`badge_solid`]. High-water: a depth already earned keeps its
/// original date even if it later drops below the mature line. Badges gate
/// nothing — this is bookkeeping only, never a lifecycle interaction. Does
/// not save.
pub fn note_badges(store: &mut Store, subject: &str, cards: &[Card], now_ms: u64) {
    for depth in [Depth::Recognize, Depth::Recall, Depth::Reconstruct] {
        if store.badge_earned(subject, depth).is_some() || !badge_solid(cards, store, depth) {
            continue;
        }
        let entry = store.decks.entry(subject.to_string()).or_default();
        match depth {
            Depth::Recognize => entry.recognized_at_ms = Some(now_ms),
            Depth::Recall => entry.recalled_at_ms = Some(now_ms),
            Depth::Reconstruct => entry.reconstructed_at_ms = Some(now_ms),
        }
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
/// Cloze edge: a multi-hole cloze block is stored as one sidecar
/// entry per hole, all sharing `parent` + the same whole-block `text`. Promoting one hole
/// appends the whole block, so the deck gains every hole as a real card — so
/// [`Store::remove_virtual_block`] drops every hole's sidecar entry, not just
/// the promoted one, leaving no orphans behind. Each hole's schedule carries
/// (its id matches its new deck sub-card).
pub fn promote_virtual(store: &mut Store, id: &str, deck_path: &Path) -> AnyResult<()> {
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

/// Splits L1 deck text into one string per top-level card block: a new block
/// begins at each `## ` front (column 0, outside a code fence); note,
/// directive, and answer lines attach to the current block. Any preamble
/// before the first front (blank lines, frontmatter, prose) is dropped; it
/// belongs to no card. Each block is verbatim source lines joined with `\n`
/// and ends with a newline.
pub fn split_card_blocks(text: &str) -> Vec<String> {
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    // Fence tracking with the parser's own rule, so a fenced `## ` line stays
    // content instead of starting a bogus block.
    let mut fence: Option<char> = None;
    for raw in text.lines() {
        match fence {
            Some(ch) => {
                if crate::l1::closes_fence(raw, ch) {
                    fence = None;
                }
            }
            None => {
                if let Some(ch) = crate::l1::fence_opener(raw) {
                    fence = Some(ch);
                } else if raw.starts_with("## ") {
                    blocks.push(vec![raw]);
                    continue;
                }
            }
        }
        if let Some(current) = blocks.last_mut() {
            current.push(raw);
        }
        // else: preamble before the first front, dropped.
    }
    blocks
        .into_iter()
        .map(|lines| format!("{}\n", lines.join("\n")))
        .collect()
}

/// Turns the cleaned remediation deck-text into virtual cards in `store`
/// (kind [`VirtualKind::Remediation`], `parent = subject`). Each top-level
/// card block gains a freshly minted `<!-- id: -->` token on its `## ` line
/// and is stored (stamped) verbatim as a [`VirtualCard::text`]; its id is the
/// `Card::id` that re-parsing that stamped block yields (a cloze block yields
/// one sidecar entry per hole, all sharing the block text but keyed by
/// distinct ids). Saves the store once after the batch. The deck file is
/// never touched. Returns how many cards were created or revived.
///
/// `deck_fingerprints` are the §7 canonical-content fingerprints of the deck's
/// authored cards ([`crate::l1::content_fingerprint`]). Dedup is by content,
/// not id: every block carries a fresh random token, so re-running remediation
/// on the same gap must recognize it by canonical content, not by a recomputed
/// id (which is always new). A block whose content already exists as a deck
/// card is skipped; one that matches only fully-retired virtual cards revives
/// them; one that matches an active virtual card is left as-is.
pub fn store_remediation_cards(
    store: &mut Store,
    subject: &str,
    deck_fingerprints: &std::collections::HashSet<u64>,
    cards_text: &str,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> AnyResult<usize> {
    let blocks = split_card_blocks(cards_text);
    if blocks.is_empty() {
        bail!("remediation produced no cards to store");
    }

    let mut created_or_revived = 0;
    for block in &blocks {
        // Mint this block's identity: the token rides the `## ` line, so the
        // stored text re-parses to the same ids forever.
        let token =
            crate::token::mint().map_err(|e| anyhow::anyhow!("cannot mint a token: {e}"))?;
        let block = stamp_block(block, &token);
        // Parse the block on its own so a cloze block yields its N sub-cards,
        // each with its own id. A malformed block is a hard error (error, never
        // fabricate) rather than a silently-dropped card.
        let cards = crate::l1::parse_str(subject, &block)?;
        let Some(first) = cards.first() else {
            continue;
        };
        // Dedup at BLOCK granularity: the parser stamps every sub-card of a block
        // with the block's §7 fingerprint (front + raw answer, `\cloze{...}`
        // markers literal), so all holes share one value. This makes the block
        // dedup and revive as a unit, and keeps a plain card that merely repeats a
        // hole's hidden text (its answer has no markers) from colliding with it.
        let fingerprint = first.content_fingerprint;
        // Already drillable as a deck card: don't shadow it with a virtual.
        if deck_fingerprints.contains(&fingerprint) {
            continue;
        }
        let existing = store.virtual_ids_with_content(subject, fingerprint);
        if existing.is_empty() {
            // New content: store every sub-card (a cloze block yields one entry
            // per hole, each with its own id) and seed a fresh schedule.
            for card in &cards {
                let Some(id) = card.id() else {
                    continue;
                };
                store.insert_virtual(VirtualCard {
                    id: id.clone(),
                    kind: VirtualKind::Remediation,
                    parent: subject.to_string(),
                    text: block.clone(),
                    created_ms: now_ms,
                });
                // Invariant (§6): records exist before the schedule entry does.
                store.ensure_records(card);
                store.get_or_insert(&id, now_ms);
                created_or_revived += 1;
            }
        } else if existing
            .iter()
            .all(|id| crate::session::is_retired_id(id, store, retire_after_days))
        {
            // A fully-retired dupe of this content: revive every matching entry
            // (reset its schedule below the cap). The sidecar text is unchanged.
            for id in &existing {
                *store.get_or_insert(id, now_ms) = CardState::new(now_ms);
                created_or_revived += 1;
            }
        }
        // Else at least one matching entry is still active: leave it, no reset.
    }
    store.save()?;
    Ok(created_or_revived)
}

/// The §7 canonical-content fingerprint of a stored virtual card: parse its
/// block under its `parent`, find the sub-card this entry stands for, and
/// fingerprint its canonical content. `None` if the stored text no longer
/// re-parses to a card carrying this entry's id (a corrupt or superseded
/// sidecar entry).
fn virtual_fingerprint(vc: &VirtualCard) -> Option<u64> {
    let cards = crate::l1::parse_str(&vc.parent, &vc.text).ok()?;
    let card = cards
        .iter()
        .find(|c| c.id().as_deref() == Some(vc.id.as_str()))?;
    // The parser carries the block-level fingerprint on every sub-card, so this is
    // the same value the candidate side compares against.
    Some(card.content_fingerprint)
}

/// Appends ` <!-- id: token -->` to a card block's `## ` front line (its first
/// line by construction, [`split_card_blocks`]), leaving every other byte
/// untouched.
fn stamp_block(block: &str, token: &str) -> String {
    match block.split_once('\n') {
        Some((front, rest)) => format!("{front} <!-- id: {token} -->\n{rest}"),
        None => format!("{block} <!-- id: {token} -->"),
    }
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

/// Syncthing conflict copies sitting next to `store_path`
/// (`<stem>.sync-conflict-*.<ext>`): evidence that two devices wrote the
/// store concurrently. Sorted for stable output; a missing directory is
/// simply no conflicts.
pub fn sync_conflicts(store_path: &Path) -> Vec<PathBuf> {
    let Some(dir) = store_path.parent() else {
        return Vec::new();
    };
    let (Some(stem), Some(ext)) = (
        store_path.file_stem().and_then(|s| s.to_str()),
        store_path.extension().and_then(|e| e.to_str()),
    ) else {
        return Vec::new();
    };
    let prefix = format!("{stem}.sync-conflict-");
    let suffix = format!(".{ext}");
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(&suffix))
        })
        .collect();
    out.sort();
    out
}

/// The label this machine stamps into stores it writes (see
/// [`Store::device`]): the trimmed content of the `device` file in `dir`,
/// created as `alix-<4 hex>` on first use. Plaintext on purpose: name your
/// machine by editing the file (e.g. to `desktop`).
pub fn device_label_in(dir: &Path) -> Option<String> {
    let path = dir.join("device");
    if let Ok(text) = std::fs::read_to_string(&path) {
        let label = text.trim();
        if !label.is_empty() {
            return Some(label.to_string());
        }
    }
    let label = generate_device_label();
    std::fs::create_dir_all(dir).ok()?;
    std::fs::write(&path, format!("{label}\n")).ok()?;
    Some(label)
}

/// [`device_label_in`] against the default alix data dir
/// (`~/.local/share/alix` on Linux).
pub fn device_label() -> Option<String> {
    let dirs = directories::ProjectDirs::from("", "", "alix")?;
    device_label_in(dirs.data_dir())
}

/// `alix-<4 hex>`: unique enough to tell one user's devices apart, from
/// std-only randomness (a randomly keyed hasher).
fn generate_device_label() -> String {
    use std::hash::{BuildHasher, Hasher};
    let r = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    format!("alix-{:04x}", r & 0xffff)
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

    /// A hole fingerprint from a `word` key and a `context` key. The realign
    /// matcher only tests equality, so distinct integers stand in for distinct
    /// hidden texts and masked lines.
    fn hf(word: u64, context: u64) -> HoleFingerprint {
        HoleFingerprint {
            text_fp: word,
            line_fp: context,
        }
    }

    #[test]
    fn inserting_a_hole_shifts_neighbors_without_losing_schedules() {
        // Holes A, B; a new hole is inserted at the front.
        let a = hf(1, 10);
        let b = hf(2, 20);
        let fresh_word = hf(9, 90);
        let outcome = realign_holes(&[a, b], &[fresh_word, a, b]);
        // A (stored 0) → file 1, B (stored 1) → file 2; file 0 is fresh.
        assert_eq!(vec![(0, 1), (1, 2)], outcome.remap);
        assert_eq!(vec![0], outcome.fresh);
        assert!(outcome.orphaned.is_empty());
    }

    #[test]
    fn deleting_a_hole_orphans_exactly_that_record() {
        // Holes A, B, C; the middle one (B) is deleted.
        let a = hf(1, 10);
        let b = hf(2, 20);
        let c = hf(3, 30);
        let outcome = realign_holes(&[a, b, c], &[a, c]);
        assert_eq!(vec![(0, 0), (2, 1)], outcome.remap);
        assert_eq!(vec![1], outcome.orphaned);
        assert!(outcome.fresh.is_empty());
    }

    #[test]
    fn reordering_holes_follows_the_words() {
        // A and B swap document positions (each keeps its own text + context,
        // e.g. two holes on different lines that trade order).
        let a = hf(1, 10);
        let b = hf(2, 20);
        let outcome = realign_holes(&[a, b], &[b, a]);
        // A's schedule (stored 0) follows to file 1; B's (stored 1) to file 0.
        assert_eq!(vec![(0, 1), (1, 0)], outcome.remap);
        assert!(outcome.orphaned.is_empty());
        assert!(outcome.fresh.is_empty());
    }

    #[test]
    fn a_context_rewrite_still_matches_by_text_alone() {
        // Same hidden word, rewritten surrounding context (line_fp differs).
        let stored = hf(1, 10);
        let rewritten = hf(1, 99);
        let outcome = realign_holes(&[stored], &[rewritten]);
        // Pass 1 (exact) misses; pass 2 (text alone) anchors it.
        assert_eq!(vec![(0, 0)], outcome.remap);
        assert!(outcome.orphaned.is_empty());
        assert!(outcome.fresh.is_empty());
    }

    #[test]
    fn identical_twins_pair_in_document_order_on_both_sides() {
        // Two indistinguishable holes (same word AND context) on each side.
        let twin = hf(5, 50);
        let outcome = realign_holes(&[twin, twin], &[twin, twin]);
        // Document order on both sides: stored 0 → file 0, stored 1 → file 1.
        // (A swap would be observationally a no-op, but the tie-break pins this.)
        assert_eq!(vec![(0, 0), (1, 1)], outcome.remap);
        assert!(outcome.orphaned.is_empty());
        assert!(outcome.fresh.is_empty());
    }

    #[test]
    fn word_and_context_both_changed_is_a_fresh_hole() {
        // Neither text nor context survives: no match at all.
        let stored = hf(1, 10);
        let changed = hf(7, 70);
        let outcome = realign_holes(&[stored], &[changed]);
        assert!(outcome.remap.is_empty());
        assert_eq!(vec![0], outcome.fresh);
        assert_eq!(vec![0], outcome.orphaned);
    }

    #[test]
    fn a_fresh_hole_always_wins_the_live_key_and_the_orphan_goes_to_the_shelf() {
        // A new word replaces the old one at the same position: the file hole
        // is fresh (it will own the live token-0 key) and the stored record
        // orphans (its schedule is evicted, never inherited by the new word).
        let stored = hf(1, 10);
        let replacement = hf(8, 80);
        let outcome = realign_holes(&[stored], &[replacement]);
        assert!(outcome.remap.is_empty());
        assert_eq!(vec![0], outcome.fresh);
        assert_eq!(vec![0], outcome.orphaned);
    }

    #[test]
    fn the_cascade_rebuilds_entries_into_a_fresh_map() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let token = "tok";
        let a = hf(1, 10);
        let b = hf(2, 20);
        store.ensure_records_raw(token, 100, &[a, b]);
        store.get_or_insert("tok-0", 0).total_reviews = 1; // hole A's schedule
        store.get_or_insert("tok-1", 0).total_reviews = 2; // hole B's schedule

        // Hole 0's word is replaced (Z); hole 1 (B) is unchanged.
        let z = hf(8, 80);
        let outcome = store.realign_card_holes(token, &[z, b], 100).unwrap();
        assert_eq!(vec![(1, 1)], outcome.remap);
        assert_eq!(vec![0], outcome.orphaned);
        assert_eq!(vec![0], outcome.fresh);

        // B's schedule stays put (a fingerprinted move wins its key)…
        assert_eq!(2, store.get("tok-1").unwrap().total_reviews);
        // …the fresh word Z owns the live token-0 key with NO inherited
        // schedule (it will get a fresh one when first reviewed)…
        assert!(store.get("tok-0").is_none());
        // …and A's orphaned schedule sits on the shelf, never under a live key.
        assert_eq!(1, store.hole_orphans().len());
        assert_eq!("tok", store.hole_orphans()[0].token);
        assert_eq!(a, store.hole_orphans()[0].fp);
        assert_eq!(1, store.hole_orphans()[0].state.total_reviews);
        // Records are refreshed to the current file holes.
        assert_eq!(vec![z, b], store.records(token).unwrap().holes);
    }

    #[test]
    fn a_stray_high_index_hole_entry_is_pulled_by_the_cascade_not_left_to_squat() {
        // A `token-N` entry above the stored-hole range (unreachable today, but
        // structurally possible) must not survive the rebuild where a future
        // fresh hole could inherit it: the cascade pulls the whole `-N` family,
        // so a stray orphans to the shelf like any other unmapped record.
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let token = "tok";
        let a = hf(1, 10);
        let b = hf(2, 20);
        store.ensure_records_raw(token, 100, &[a, b]);
        store.get_or_insert("tok-0", 0).total_reviews = 1; // hole a
        store.get_or_insert("tok-1", 0).total_reviews = 2; // hole b
        store.get_or_insert("tok-5", 0).total_reviews = 9; // a stray, out of range

        // Reorder the two live holes; nothing orphans in the pure cascade.
        let outcome = store.realign_card_holes(token, &[b, a], 100).unwrap();
        assert_eq!(vec![(0, 1), (1, 0)], outcome.remap);

        // The two live schedules followed their words…
        assert_eq!(2, store.get("tok-0").unwrap().total_reviews, "b -> hole 0");
        assert_eq!(1, store.get("tok-1").unwrap().total_reviews, "a -> hole 1");
        // …and the stray is gone from every live key, evicted to the shelf.
        assert!(
            store.get("tok-5").is_none(),
            "the stray must not survive under a live key"
        );
        assert!(
            store
                .hole_orphans()
                .iter()
                .any(|o| o.token == token && o.state.total_reviews == 9),
            "the stray's schedule should sit on the shelf"
        );
    }

    #[test]
    fn a_stale_fingerprint_version_is_ignored_and_rewritten_never_mismatched() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let token = "tok";
        let a = hf(1, 10);
        let b = hf(2, 20);
        // Records written under a superseded FP_VERSION, with holes that would
        // orphan tok-0 if a cascade ran.
        store.records.insert(
            token.to_string(),
            CardRecords {
                version: FP_VERSION.wrapping_add(1),
                content_fp: 100,
                holes: vec![a],
            },
        );
        store.get_or_insert("tok-0", 0).total_reviews = 7;

        // A stale version must be ignored (never mismatched into a cascade that
        // would wipe schedules on an unchanged card), and simply rewritten.
        let outcome = store.realign_card_holes(token, &[a, b], 100);
        assert!(outcome.is_none());
        // The positional schedule is untouched; nothing was evicted.
        assert_eq!(7, store.get("tok-0").unwrap().total_reviews);
        assert!(store.hole_orphans().is_empty());
        // Records are rewritten at the current version + current file holes.
        let rec = store.records(token).unwrap();
        assert_eq!(FP_VERSION, rec.version);
        assert_eq!(vec![a, b], rec.holes);
    }

    #[test]
    fn orphans_are_the_keys_with_no_live_card_or_deck_and_prune_clears_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        // Live card + orphaned card key; a virtual card's schedule key is NOT
        // an orphan (it is a legitimate local card with no deck file).
        store.get_or_insert("live", 0);
        store.get_or_insert("gone", 0);
        store.insert_virtual(VirtualCard {
            id: "vq".to_string(),
            kind: VirtualKind::Remediation,
            parent: "rust.md".to_string(),
            text: "## v <!-- id: vq -->\nb\n".to_string(),
            created_ms: 0,
        });
        store.get_or_insert("vq", 0); // the virtual card's own schedule
        // Live deck subject + an orphaned one (a renamed/deleted deck file).
        store.set_last_depth("rust.md", Depth::Recall);
        store.set_last_depth("deleted.md", Depth::Recall);

        let known_cards: HashSet<String> = ["live".to_string()].into_iter().collect();
        let known_subjects: HashSet<String> = ["rust.md".to_string()].into_iter().collect();
        let orphans = store.orphans(&known_cards, &known_subjects);
        assert_eq!(vec!["gone".to_string()], orphans.cards);
        assert_eq!(vec!["deleted.md".to_string()], orphans.decks);
        assert_eq!(2, orphans.len());

        // Pruning drops exactly the orphaned keys, leaving the live ones and
        // the virtual schedule untouched.
        assert_eq!(2, store.prune_orphans(&orphans));
        assert!(store.get("live").is_some());
        assert!(store.get("vq").is_some());
        assert_eq!(Some(Depth::Recall), store.last_depth("rust.md"));
        assert!(store.get("gone").is_none());
        assert_eq!(None, store.last_depth("deleted.md"));
    }

    #[test]
    fn reset_orphans_clears_shelf_entries_and_records_of_pruned_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let a = hf(1, 10);

        // An orphan token carrying all three families (schedule, records, a shelf
        // entry), claimed by no live card.
        store.get_or_insert("gonetoken", 0).total_reviews = 3;
        store.ensure_records_raw("gonetoken", 100, &[a]);
        store.hole_orphans.push(OrphanedHole {
            token: "gonetoken".to_string(),
            fp: a,
            state: CardState::new(0),
        });
        // A live token carrying the same three families, claimed by a live card.
        store.get_or_insert("livetoken", 0).total_reviews = 7;
        store.ensure_records_raw("livetoken", 200, &[a]);
        store.hole_orphans.push(OrphanedHole {
            token: "livetoken".to_string(),
            fp: a,
            state: CardState::new(0),
        });

        let known_cards: HashSet<String> = ["livetoken".to_string()].into_iter().collect();
        let orphans = store.orphans(&known_cards, &HashSet::new());
        assert_eq!(vec!["gonetoken".to_string()], orphans.cards);

        store.prune_orphans(&orphans);

        // The pruned token's schedule, records, and shelf entry are all gone: with
        // no schedule left, its records and shelf entry are unreclaimable dead weight.
        assert!(store.get("gonetoken").is_none());
        assert!(store.records("gonetoken").is_none());
        assert!(store.hole_orphans().iter().all(|o| o.token != "gonetoken"));
        // The live token keeps all three: its shelf entry stays as evidence for a
        // future per-hole reclaim.
        assert!(store.get("livetoken").is_some());
        assert!(store.records("livetoken").is_some());
        assert!(store.hole_orphans().iter().any(|o| o.token == "livetoken"));
    }

    #[test]
    fn wipe_deck_clears_every_family_for_its_tokens_and_spares_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let a = hf(1, 10);

        // The doomed deck's token: a plain schedule, a hole schedule, records, a
        // shelf entry, its deck-family mastery, and a virtual card parented to it.
        store.get_or_insert("doom", 0);
        store.get_or_insert("doom-0", 0);
        store.ensure_records_raw("doom", 100, &[a]);
        store.hole_orphans.push(OrphanedHole {
            token: "doom".to_string(),
            fp: a,
            state: CardState::new(0),
        });
        store.set_deck_mastered("doomed.md", 1);
        store.insert_virtual(VirtualCard {
            id: "vdoom".to_string(),
            kind: VirtualKind::Remediation,
            parent: "doomed.md".to_string(),
            text: "## v <!-- id: vdoom -->\nx\n".to_string(),
            created_ms: 0,
        });
        store.get_or_insert("vdoom", 0);
        // A bystander token with the same families, under another deck.
        store.get_or_insert("keep", 0);
        store.ensure_records_raw("keep", 200, &[a]);
        store.hole_orphans.push(OrphanedHole {
            token: "keep".to_string(),
            fp: a,
            state: CardState::new(0),
        });
        store.set_deck_mastered("keep.md", 1);

        let tokens: HashSet<String> = ["doom".to_string()].into_iter().collect();
        let wiped = store.wipe_deck(&tokens, "doomed.md");

        // Both the base and the hole schedule counted; every doomed family gone.
        assert_eq!(2, wiped);
        assert!(store.get("doom").is_none());
        assert!(store.get("doom-0").is_none());
        assert!(store.records("doom").is_none());
        assert!(store.hole_orphans().iter().all(|o| o.token != "doom"));
        assert!(!store.deck_mastered("doomed.md"));
        assert!(store.get_virtual("vdoom").is_none());
        assert!(store.get("vdoom").is_none());
        // The bystander deck is untouched across every family.
        assert!(store.get("keep").is_some());
        assert!(store.records("keep").is_some());
        assert!(store.hole_orphans().iter().any(|o| o.token == "keep"));
        assert!(store.deck_mastered("keep.md"));
    }

    #[test]
    fn save_stamps_the_writer_and_a_reopen_sees_it_as_foreign_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut store = Store::open(&path).unwrap();
        store.device = Some("desk-1".into());
        store.save().unwrap();

        let reopened = Store::open(&path).unwrap();
        let (device, _) = reopened
            .foreign_writer("phone-1", crate::time::now_ms())
            .expect("another device sees the marker");
        assert_eq!(device, "desk-1");
        assert!(
            reopened
                .foreign_writer("desk-1", crate::time::now_ms())
                .is_none(),
            "a device's own writes are not foreign"
        );
    }

    #[test]
    fn an_unnamed_save_preserves_the_existing_writer_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut store = Store::open(&path).unwrap();
        store.device = Some("desk-1".into());
        store.save().unwrap();

        // A consumer with no device (a test, a one-off tool) saves without
        // erasing who really wrote last.
        let unnamed = Store::open(&path).unwrap();
        unnamed.save().unwrap();
        let reopened = Store::open(&path).unwrap();
        let (device, _) = reopened
            .foreign_writer("phone-1", crate::time::now_ms())
            .expect("the marker survives an unnamed save");
        assert_eq!(device, "desk-1");
    }

    #[test]
    fn a_store_without_a_writer_marker_loads_and_reports_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(&path, r#"{"version":1,"cards":{}}"#).unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.foreign_writer("phone-1", 0).is_none());
    }

    #[test]
    fn the_warn_window_separates_roaming_from_concurrent_writes() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        store.last_writer = Some(Writer {
            device: "desk-1".into(),
            at_ms: 1_000,
        });
        let just_inside = 1_000 + FOREIGN_WRITE_WARN_WINDOW_MS - 1;
        let at_the_edge = 1_000 + FOREIGN_WRITE_WARN_WINDOW_MS;
        assert!(
            store
                .recent_foreign_writer("phone-1", just_inside)
                .is_some()
        );
        assert!(
            store
                .recent_foreign_writer("phone-1", at_the_edge)
                .is_none(),
            "an old write is ordinary roaming, not a warning"
        );
        assert!(store.recent_foreign_writer("desk-1", just_inside).is_none());
    }

    #[test]
    fn sync_conflicts_finds_syncthing_copies_and_ignores_near_misses() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("progress.json");
        std::fs::write(&store_path, "{}").unwrap();
        let conflict = dir
            .path()
            .join("progress.sync-conflict-20260714-101112-ABCDEF7.json");
        std::fs::write(&conflict, "{}").unwrap();
        for near_miss in [
            "recent.sync-conflict-20260714-101112-AAAAAAA.json",
            "progress.sync-conflict-20260714.txt",
            "progress.json.tmp",
        ] {
            std::fs::write(dir.path().join(near_miss), "{}").unwrap();
        }
        assert_eq!(sync_conflicts(&store_path), vec![conflict]);
        assert_eq!(
            sync_conflicts(&dir.path().join("missing/progress.json")),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn device_label_is_created_once_and_stays_editable() {
        let dir = tempfile::tempdir().unwrap();
        let first = device_label_in(dir.path()).unwrap();
        assert!(
            first.starts_with("alix-") && first.len() == 9,
            "generated shape: {first}"
        );
        assert_eq!(
            device_label_in(dir.path()).unwrap(),
            first,
            "stable across calls"
        );
        std::fs::write(dir.path().join("device"), "desktop\n").unwrap();
        assert_eq!(device_label_in(dir.path()).unwrap(), "desktop");
    }

    #[test]
    fn open_rejects_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(&path, "this is not json").unwrap();
        assert!(Store::open(&path).is_err());
    }

    #[test]
    fn open_keeps_a_card_key_of_any_charset() {
        // Card keys are identity tokens now, so a key that isn't the canonical
        // token charset passes through verbatim (it becomes doctor material)
        // rather than failing the whole load, which would refuse a user's real
        // progress.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(
            &path,
            r#"{"version":1,"cards":{"not-a-token":{"acquired_ms":0}}}"#,
        )
        .unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.get("not-a-token").is_some());
    }

    #[test]
    fn last_review_ms_is_the_latest_across_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut store = Store::open(&path).unwrap();
        assert_eq!(None, store.last_review_ms());
        store
            .get_or_insert("1", 0)
            .record_review(100, Grade::Pass, Depth::Recall, false);
        store
            .get_or_insert("2", 0)
            .record_review(300, Grade::Pass, Depth::Recall, false);
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
        let state = store.get_or_insert("42", 1000);
        state.record_review(1000, Grade::Pass, Depth::Recall, false);
        store.save().unwrap();

        let reloaded = Store::open(&path).unwrap();
        assert_eq!(1, reloaded.len());
        let state = reloaded.get("42").unwrap();
        assert_eq!(1, state.total_reviews);
        assert_eq!(
            vec![Review {
                ts_ms: 1000,
                grade: Grade::Pass,
                depth: Depth::Recall,
                propagated: false
            }],
            state.history
        );
    }

    #[test]
    fn propagated_flag_survives_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");

        let mut store = Store::open(&path).unwrap();
        let state = store.get_or_insert("42", 1000);
        state.record_review(1000, Grade::Pass, Depth::Reconstruct, false);
        state.record_review(1000, Grade::Pass, Depth::Recall, true);
        store.save().unwrap();

        // The marker round-trips; the unmarked review doesn't serialize the key
        // at all (no store bloat for the common case).
        let json = std::fs::read_to_string(&path).unwrap();
        assert_eq!(1, json.matches("propagated").count());

        let reloaded = Store::open(&path).unwrap();
        let history = &reloaded.get("42").unwrap().history;
        assert!(!history[0].propagated);
        assert!(history[1].propagated);
        assert_eq!(Depth::Recall, history[1].depth);
    }

    #[test]
    fn deck_mastered_roundtrips_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");

        let mut store = Store::open(&path).unwrap();
        assert!(!store.deck_mastered("rust.md"));
        assert_eq!(None, store.deck_mastered_at("rust.md"));
        store.set_deck_mastered("rust.md", 1234);
        assert!(store.deck_mastered("rust.md"));
        assert_eq!(Some(1234), store.deck_mastered_at("rust.md"));
        store.save().unwrap();

        // Survives a save/reload.
        let mut reloaded = Store::open(&path).unwrap();
        assert!(reloaded.deck_mastered("rust.md"));
        assert_eq!(Some(1234), reloaded.deck_mastered_at("rust.md"));
        // Per-deck clear drops just that deck.
        assert!(reloaded.clear_deck_mastered("rust.md"));
        assert!(!reloaded.deck_mastered("rust.md"));
        assert!(!reloaded.clear_deck_mastered("rust.md")); // nothing left
    }

    #[test]
    fn exam_failed_records_and_a_pass_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");

        let mut store = Store::open(&path).unwrap();
        assert_eq!(None, store.exam_failed_at("t.md"));
        // A failed exam stamps the cooldown without mastering the deck.
        store.set_exam_failed("t.md", 5000);
        assert_eq!(Some(5000), store.exam_failed_at("t.md"));
        assert!(!store.deck_mastered("t.md"));
        store.save().unwrap();

        // Survives a save/reload.
        let mut reloaded = Store::open(&path).unwrap();
        assert_eq!(Some(5000), reloaded.exam_failed_at("t.md"));
        // A later pass masters the deck and clears the cooldown.
        reloaded.set_deck_mastered("t.md", 9000);
        assert!(reloaded.deck_mastered("t.md"));
        assert_eq!(None, reloaded.exam_failed_at("t.md"));
    }

    #[test]
    fn per_deck_clear_drops_the_cooldown_too() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.set_exam_failed("t.md", 1);
        assert!(store.clear_deck_mastered("t.md"));
        assert_eq!(None, store.exam_failed_at("t.md"));
    }

    #[test]
    fn cooldown_remaining_is_none_for_a_subject_that_never_failed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("p.json")).unwrap();
        assert_eq!(None, cooldown_remaining_ms(&store, "t.md", 3600, 0));
    }

    #[test]
    fn cooldown_remaining_is_none_when_the_cooldown_is_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.set_exam_failed("t.md", 1_000);
        assert_eq!(None, cooldown_remaining_ms(&store, "t.md", 0, 1_030_000));
    }

    #[test]
    fn cooldown_remaining_reports_the_exact_ms_left_inside_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.set_exam_failed("t.md", 1_000);
        // 1h cooldown, 30s after the fail -> the rest of the hour remains.
        let now = 1_000 + 30_000;
        assert_eq!(
            Some(3_600_000 - 30_000),
            cooldown_remaining_ms(&store, "t.md", 3600, now)
        );
    }

    #[test]
    fn cooldown_remaining_is_none_once_the_window_has_elapsed() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.set_exam_failed("t.md", 1_000);
        assert_eq!(
            None,
            cooldown_remaining_ms(&store, "t.md", 3600, 1_000 + 3_600_001)
        );
    }

    #[test]
    fn loads_a_v1_deck_record_with_a_bare_mastered_timestamp() {
        // Pre-v2 stores wrote `mastered_at_ms` as a bare number (not optional);
        // it must still load as a mastered deck.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(
            &path,
            "{\"version\":1,\"cards\":{},\"decks\":{\"rust.md\":{\"mastered_at_ms\":1234}}}",
        )
        .unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.deck_mastered("rust.md"));
        assert_eq!(Some(1234), store.deck_mastered_at("rust.md"));
        assert_eq!(None, store.exam_failed_at("rust.md"));
    }

    #[test]
    fn clear_also_drops_deck_mastered() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.set_deck_mastered("a.md", 1);
        store.clear();
        assert!(!store.deck_mastered("a.md"));
    }

    #[test]
    fn loads_store_file_without_decks_field() {
        // A store written before the `decks` field existed must still load.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(&path, "{\"version\":1,\"cards\":{}}").unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.is_empty());
        assert!(!store.deck_mastered("anything.md"));
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
            r#"{"version":999,"cards":{"5":{"acquired_ms":7,"history":[{"ts_ms":100,"passed":false}]}}}"#,
        )
        .unwrap();
        let store = Store::open(&path).unwrap();
        let state = store.get("5").unwrap();
        assert_eq!(7, state.acquired_ms); // scheduling state survives
        assert_eq!(100, state.history[0].ts_ms);
        assert_eq!(Grade::Pass, state.history[0].grade); // old `passed` dropped → default
    }

    #[test]
    fn save_writes_the_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut store = Store::open(&path).unwrap();
        store.get_or_insert("1", 0);
        store.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(&format!("\"version\": {CURRENT_VERSION}")));
    }

    #[test]
    fn remove_drops_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert("42", 1000);
        assert!(store.remove("42"));
        assert!(store.get("42").is_none());
        // Removing again reports nothing was there.
        assert!(!store.remove("42"));
    }

    #[test]
    fn clear_empties_and_counts() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert("1", 0);
        store.get_or_insert("2", 0);
        assert_eq!(2, store.clear());
        assert!(store.is_empty());
        assert_eq!(0, store.clear()); // already empty
    }

    /// The canonical one-card deck-format `text` of a sample virtual card.
    const BORROW_TEXT: &str = "## What does the borrow checker enforce? <!-- id: vb1 -->\nExactly one mutable borrow, or many shared ones\n";

    /// Builds a virtual card from its canonical deck-format `text` under
    /// `parent`, deriving its id exactly as the substrate does — the `Card::id`
    /// of the (plain) card that `parse(parent, text)` yields. Seeds no schedule;
    /// a caller that needs the card scheduled adds a `store.cards` entry itself.
    fn virtual_card(parent: &str, text: &str) -> VirtualCard {
        let id = crate::l1::parse_str(parent, text).unwrap()[0].id().unwrap();
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
        let vc = virtual_card("rust.md", BORROW_TEXT);
        let id = vc.id.clone();

        store.insert_virtual(vc);

        let got = store.get_virtual(&id).unwrap();
        assert_eq!("rust.md", got.parent);
        assert_eq!(VirtualKind::Remediation, got.kind);
        assert_eq!(BORROW_TEXT, got.text);
        assert!(store.is_virtual(&id));
    }

    #[test]
    fn virtual_card_survives_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut store = Store::open(&path).unwrap();
        let vc = virtual_card("rust.md", BORROW_TEXT);
        let id = vc.id.clone();
        store.insert_virtual(vc.clone());
        store.save().unwrap();

        let reloaded = Store::open(&path).unwrap();
        let got = reloaded.get_virtual(&id).unwrap();
        assert_eq!(&vc, got);
    }

    #[test]
    fn virtual_cards_for_matches_on_parent_subject() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        // Distinct ids come from distinct content (different back / subject).
        store.insert_virtual(virtual_card("rust.md", "## f <!-- id: v1 -->\nback one\n"));
        store.insert_virtual(virtual_card("rust.md", "## f <!-- id: v2 -->\nback two\n"));
        store.insert_virtual(virtual_card("other.md", "## f <!-- id: v3 -->\nback one\n"));

        let rust_cards = store.virtual_cards_for("rust.md");
        assert_eq!(2, rust_cards.len());
        assert!(rust_cards.iter().all(|v| v.parent == "rust.md"));

        assert_eq!(1, store.virtual_cards_for("other.md").len());
        assert!(store.virtual_cards_for("nonexistent.md").is_empty());
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
        assert!(store.get_virtual("123").is_none());
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
                "cards":{"5":{"acquired_ms":7}},
                "decks":{"rust.md":{"mastered_at_ms":1234}},
                "virtual_cards":{
                    "v:abc":{"id":"v:abc","kind":"Remediation","parent":"rust.md",
                             "content":{"front":"f","back":["b"],"mode":null},
                             "state":{"acquired_ms":0},"created_ms":0}
                }}"#,
        )
        .unwrap();
        let store = Store::open(&path).unwrap();
        // Real progress survives …
        assert_eq!(7, store.get("5").unwrap().acquired_ms);
        assert!(store.deck_mastered("rust.md"));
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
        let deck_path = write_deck(
            dir.path(),
            "rust.md",
            "## existing <!-- id: ex1 -->\nanswer\n",
        );
        let store_path = dir.path().join("progress.json");
        let mut store = Store::open(&store_path).unwrap();
        let vc = virtual_card("rust.md", BORROW_TEXT);
        let id = vc.id.clone();
        store.insert_virtual(vc);

        promote_virtual(&mut store, &id, &deck_path).unwrap();

        assert!(store.get_virtual(&id).is_none());

        let text = std::fs::read_to_string(&deck_path).unwrap();
        let cards = crate::l1::parse_str("rust.md", &text).unwrap();
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
        assert!(reloaded.get_virtual(&id).is_none());
    }

    #[test]
    fn promote_leaves_existing_deck_card_ids_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = write_deck(
            dir.path(),
            "rust.md",
            "## one <!-- id: q1 -->\n1\n\n## two <!-- id: q2 -->\n2\n",
        );
        let before =
            crate::l1::parse_str("rust.md", &std::fs::read_to_string(&deck_path).unwrap()).unwrap();
        let ids_before: Vec<String> = before.iter().map(|c| c.id().unwrap()).collect();

        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let vc = virtual_card("rust.md", BORROW_TEXT);
        let id = vc.id.clone();
        store.insert_virtual(vc);

        promote_virtual(&mut store, &id, &deck_path).unwrap();

        let after =
            crate::l1::parse_str("rust.md", &std::fs::read_to_string(&deck_path).unwrap()).unwrap();
        let ids_after: Vec<String> = after.iter().take(2).map(|c| c.id().unwrap()).collect();
        assert_eq!(ids_before, ids_after);
        assert_eq!(3, after.len()); // the two originals plus the promoted card
    }

    #[test]
    fn promote_unknown_id_errors_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = write_deck(dir.path(), "d.md", "## one\n1\n");
        let deck_before = std::fs::read_to_string(&deck_path).unwrap();
        let store_path = dir.path().join("progress.json");
        let mut store = Store::open(&store_path).unwrap();

        let result = promote_virtual(&mut store, "999", &deck_path);

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
        let deck_path = write_deck(
            dir.path(),
            "rust.md",
            "## existing <!-- id: ex1 -->\nanswer\n",
        );
        let store_path = dir.path().join("progress.json");
        let mut store = Store::open(&store_path).unwrap();

        let text = "## Complete the quote <!-- id: vcz1 -->\nTo \\cloze{be} or not to \\cloze{be}\n> Hamlet\n";
        let cards = crate::l1::parse_str("rust.md", text).unwrap();
        assert_eq!(2, cards.len());
        let id0 = cards[0].id().unwrap();
        let id1 = cards[1].id().unwrap();
        assert_ne!(id0, id1, "the two holes must have distinct ids");

        for id in [id0.clone(), id1.clone()] {
            store.insert_virtual(VirtualCard {
                id: id.clone(),
                kind: VirtualKind::Remediation,
                parent: "rust.md".to_string(),
                text: text.to_string(),
                created_ms: 1000,
            });
            // Seed a drilled schedule for each hole — promote must preserve these.
            store
                .get_or_insert(&id, 1000)
                .record_review(1000, Grade::Pass, Depth::Recall, false);
        }

        promote_virtual(&mut store, &id0, &deck_path).unwrap();

        // Both sidecar entries are gone — no orphan left for the sibling hole.
        assert!(store.get_virtual(&id0).is_none());
        assert!(store.get_virtual(&id1).is_none());
        // Both schedules survive: the promoted deck cards inherit their drilled
        // history for free.
        assert!(store.get(&id0).is_some());
        assert!(store.get(&id1).is_some());

        // The deck file gained the cloze card (both holes, since a cloze
        // promotes as one block).
        let deck_text = std::fs::read_to_string(&deck_path).unwrap();
        let deck_cards = crate::l1::parse_str("rust.md", &deck_text).unwrap();
        assert_eq!(3, deck_cards.len()); // the existing plain card + 2 cloze holes

        // A second promote of the sibling hole now bails cleanly (its sidecar
        // entry is already gone) instead of re-appending the block and
        // duplicating ids in the deck file.
        let deck_before_second = std::fs::read_to_string(&deck_path).unwrap();
        let second = promote_virtual(&mut store, &id1, &deck_path);
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
        let deck_path = write_deck(
            dir.path(),
            "rust.md",
            "## existing <!-- id: ex1 -->\nanswer\n",
        );
        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let vc = virtual_card("rust.md", BORROW_TEXT);
        let id = vc.id.clone();
        store.insert_virtual(vc);

        // Drill the schedule in `store.cards`, not on the virtual entry.
        let mut state = CardState::new(1000);
        state.record_review(1000, Grade::Pass, Depth::Recall, false);
        state.record_review(2000, Grade::Pass, Depth::Recall, false);
        state.recall = Some(FsrsState {
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
        *store.get_or_insert(&id, 1000) = state.clone();

        promote_virtual(&mut store, &id, &deck_path).unwrap();

        assert!(store.get_virtual(&id).is_none());

        let text = std::fs::read_to_string(&deck_path).unwrap();
        let cards = crate::l1::parse_str("rust.md", &text).unwrap();
        let promoted = cards
            .iter()
            .find(|c| c.front == "What does the borrow checker enforce?")
            .expect("promoted card present");
        // The id was unified at the source: the appended deck card hashes to the
        // very id the schedule was already keyed under.
        assert_eq!(Some(id), promoted.id());
        let carried = store
            .get(&promoted.id().unwrap())
            .expect("schedule carried over");
        assert_eq!(&state, carried);
    }

    #[test]
    fn a_tutor_virtual_card_round_trips_through_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut store = Store::open(&path).unwrap();
        let text = "## capital of france <!-- id: cap1 -->\nParis\n".to_string();
        let id = crate::l1::parse_str("geo.md", &text).unwrap()[0]
            .id()
            .unwrap();
        store.insert_virtual(VirtualCard {
            id: id.clone(),
            kind: VirtualKind::Tutor,
            parent: "geo.md".to_string(),
            text,
            created_ms: 5,
        });
        store.save().unwrap();

        let reopened = Store::open(&path).unwrap();
        let vc = reopened.get_virtual(&id).expect("tutor card should load");
        assert_eq!(vc.kind, VirtualKind::Tutor);
    }

    #[test]
    fn history_is_capped() {
        let mut state = CardState::new(0);
        for i in 0..(HISTORY_CAP as u64 + 10) {
            state.record_review(i, Grade::Pass, Depth::Recall, false);
        }
        assert_eq!(HISTORY_CAP, state.history.len());
        assert_eq!(10, state.history[0].ts_ms);
        assert_eq!(HISTORY_CAP as u32 + 10, state.total_reviews);
    }

    #[test]
    fn streak_resets_on_fail() {
        let mut state = CardState::new(0);
        state.record_review(1, Grade::Pass, Depth::Recall, false);
        state.record_review(2, Grade::Pass, Depth::Recall, false);
        assert_eq!(2, state.streak);
        state.record_review(3, Grade::Fail, Depth::Recall, false);
        assert_eq!(0, state.streak);
        assert_eq!(2, state.total_passes);
        assert_eq!(3, state.total_reviews);
    }

    #[test]
    fn record_review_stores_the_grade_and_partial_counts_as_a_pass() {
        let mut state = CardState::new(0);
        state.record_review(10, Grade::Partial, Depth::Recall, false);
        assert_eq!(Grade::Partial, state.history.last().unwrap().grade);
        assert_eq!(1, state.total_reviews);
        assert_eq!(1, state.total_passes); // Partial (a weak success) counts as a pass
        assert_eq!(1, state.streak);
    }

    #[test]
    fn recall_and_reconstruct_schedules_are_independent() {
        let mut s = CardState::new(1_000);
        *s.schedule_slot(Depth::Recall).unwrap() = Some(FsrsState {
            stability: 30.0,
            ..Default::default()
        });
        assert!(s.schedule(Depth::Recall).is_some());
        assert!(
            s.schedule(Depth::Reconstruct).is_none(),
            "no cross-crediting: reconstruct starts empty"
        );
        assert!(
            s.schedule(Depth::Recognize).is_none(),
            "recognize is never scheduled"
        );
    }

    #[test]
    fn per_depth_schedules_and_recognized_flag_survive_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("store.json")).unwrap();
        let st = store.get_or_insert("7", 1_000);
        *st.schedule_slot(Depth::Reconstruct).unwrap() = Some(FsrsState {
            stability: 4.5,
            ..Default::default()
        });
        st.recognized_ms = Some(2_000);
        st.record_review(2_000, Grade::Pass, Depth::Reconstruct, false);
        store.save().unwrap();
        let reloaded = Store::open(dir.path().join("store.json")).unwrap();
        let st = reloaded.get("7").unwrap();
        assert_eq!(
            Some(4.5),
            st.schedule(Depth::Reconstruct).map(|f| f.stability)
        );
        assert_eq!(Some(2_000), st.recognized_ms);
        assert_eq!(Depth::Reconstruct, st.history[0].depth);
    }

    #[test]
    fn a_pre_depth_store_loads_with_empty_schedules() {
        // Clean break: the old `fsrs` key is ignored, not aliased.
        let json = r#"{"version":1,"cards":{"7":{"acquired_ms":5,"fsrs":{"stability":9.0,"difficulty":5.0,"reps":3,"lapses":0,"state":2,"scheduled_days":9,"last_review_ms":1,"due_ms":2},"total_reviews":3}}}"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        std::fs::write(&path, json).unwrap();
        let store = Store::open(&path).unwrap();
        let st = store.get("7").unwrap();
        assert_eq!(5, st.acquired_ms, "known fields still load");
        assert!(
            st.schedule(Depth::Recall).is_none(),
            "old fsrs key is dropped, not aliased"
        );
    }

    #[test]
    fn card_state_round_trips_without_a_stage_field() {
        // The new shape carries no `stage`; `acquired_ms` defaults when absent.
        let json = r#"{"acquired_ms":1234,"fsrs":null}"#;
        let s: CardState = serde_json::from_str(json).unwrap();
        assert_eq!(s.acquired_ms, 1234);
        assert!(s.recall.is_none());
        // A legacy store carrying a stray `stage` key still loads (serde ignores it).
        let legacy = r#"{"stage":3,"acquired_ms":5}"#;
        let s2: CardState = serde_json::from_str(legacy).unwrap();
        assert_eq!(s2.acquired_ms, 5);
    }

    #[test]
    fn history_grades_survive_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut store = Store::open(&path).unwrap();
        let st = store.get_or_insert("7", 0);
        st.record_review(100, Grade::Partial, Depth::Recall, false);
        st.record_review(200, Grade::Fail, Depth::Recall, false);
        store.save().unwrap();

        let reloaded = Store::open(&path).unwrap();
        let history = &reloaded.get("7").unwrap().history;
        assert_eq!(Grade::Partial, history[0].grade);
        assert_eq!(Grade::Fail, history[1].grade);
    }

    #[test]
    fn fsrs_state_survives_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut store = Store::open(&path).unwrap();
        store.get_or_insert("9", 0).recall = Some(FsrsState {
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
        let f = reloaded.get("9").unwrap().recall.unwrap();
        assert_eq!(2000, f.due_ms);
        assert_eq!(1, f.learning_goods);
    }

    /// Parses a tiny two-card deck for the badge tests below.
    fn two_cards() -> Vec<crate::card::Card> {
        crate::l1::parse_str(
            "t.md",
            "## a <!-- id: q1 -->\n1\n\n## b <!-- id: q2 -->\n2\n",
        )
        .unwrap()
    }

    #[test]
    fn a_deck_with_all_mature_recall_cards_is_recall_solid() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let cards = two_cards();
        for card in &cards {
            store.get_or_insert(&card.id().unwrap(), 0).recall = Some(FsrsState {
                stability: 30.0,
                ..Default::default()
            });
        }
        assert!(badge_solid(&cards, &store, Depth::Recall));
    }

    #[test]
    fn badge_solid_on_an_empty_deck_is_never_solid() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("p.json")).unwrap();
        assert!(!badge_solid(&[], &store, Depth::Recall));
        assert!(!badge_solid(&[], &store, Depth::Recognize));
    }

    #[test]
    fn a_lapsed_card_drops_solid_but_keeps_the_earn_date() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let cards = two_cards();
        for card in &cards {
            store.get_or_insert(&card.id().unwrap(), 0).recall = Some(FsrsState {
                stability: 30.0,
                ..Default::default()
            });
        }
        note_badges(&mut store, "t.md", &cards, 1_000);
        assert_eq!(Some(1_000), store.badge_earned("t.md", Depth::Recall));

        // One card lapses back below the mature line.
        store.get_or_insert(&cards[0].id().unwrap(), 0).recall = Some(FsrsState {
            stability: 3.0,
            ..Default::default()
        });

        assert!(!badge_solid(&cards, &store, Depth::Recall));
        // The earn date is a high-water mark: it survives the lapse.
        assert_eq!(Some(1_000), store.badge_earned("t.md", Depth::Recall));
    }

    #[test]
    fn recognize_badge_needs_every_card_recognized() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let cards = two_cards();
        store
            .get_or_insert(&cards[0].id().unwrap(), 0)
            .recognized_ms = Some(500);
        assert!(
            !badge_solid(&cards, &store, Depth::Recognize),
            "second card not yet recognized"
        );

        store
            .get_or_insert(&cards[1].id().unwrap(), 0)
            .recognized_ms = Some(600);
        assert!(badge_solid(&cards, &store, Depth::Recognize));
    }

    #[test]
    fn last_depth_roundtrips_through_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.json");
        let mut store = Store::open(&path).unwrap();
        assert_eq!(None, store.last_depth("t.md"));

        store.set_last_depth("t.md", Depth::Reconstruct);
        assert_eq!(Some(Depth::Reconstruct), store.last_depth("t.md"));
        store.save().unwrap();

        let reloaded = Store::open(&path).unwrap();
        assert_eq!(Some(Depth::Reconstruct), reloaded.last_depth("t.md"));
    }

    #[test]
    fn mint_tutor_card_inserts_a_tutor_virtual_card() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let id = mint_tutor_card(
            &mut store,
            "geo.md",
            "capital of france",
            &["Paris".to_string()],
            100,
            &HashSet::new(),
        )
        .unwrap();
        assert!(store.is_virtual(&id));
        assert!(store.get_virtual(&id).is_some());
        // The seeded schedule (so it enters the queue as a new card) is exercised
        // end-to-end by the tests/api.rs round-trip in Task 9, where drillability
        // is asserted against the running server.
    }

    #[test]
    fn records_exist_whenever_an_entry_is_created() {
        // The §6 invariant, at the two store-side entry-creation paths: a tutor
        // mint and a remediation mint both write records alongside the schedule,
        // so no store entry is ever token→schedule with no content to reclaim
        // against. (The trace-walk grade path and review-open are covered by
        // their own tests in trace.rs / assemble.rs.)
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let tutor = mint_tutor_card(
            &mut store,
            "geo.md",
            "capital of italy?",
            &["Rome".to_string()],
            100,
            &HashSet::new(),
        )
        .unwrap();
        assert!(store.get(&tutor).is_some(), "the schedule entry exists");
        let rec = store.records(&tutor).expect("records exist for the entry");
        assert_eq!(FP_VERSION, rec.version);
        assert!(rec.holes.is_empty(), "a plain tutor card has no holes");

        store_remediation(
            &mut store,
            "d.md",
            "## Why does X happen?\nbecause Y\n",
            200,
            None,
        )
        .unwrap();
        let gap = store.virtual_cards_for("d.md")[0].id.clone();
        assert!(store.get(&gap).is_some());
        assert!(
            store.records(&gap).is_some(),
            "a remediation mint writes records too"
        );

        // A cloze remediation block records every hole under its base token.
        store_remediation(
            &mut store,
            "d.md",
            "## Fill\nthe \\cloze{a} and \\cloze{b}\n",
            300,
            None,
        )
        .unwrap();
        let cloze_id = store
            .virtual_cards_for("d.md")
            .into_iter()
            .find(|v| v.text.contains("\\cloze"))
            .unwrap()
            .id
            .clone();
        let (base, _, _) = crate::token::parse_card_id(&cloze_id).unwrap();
        assert_eq!(
            2,
            store.records(base).unwrap().holes.len(),
            "both holes recorded under the base token"
        );
    }

    #[test]
    fn a_double_tutor_mint_reports_duplicate() {
        // The dedup is by canonical content (§7), not id: a mint carries a fresh
        // random token, so minting the same card twice is caught by its content
        // fingerprint, not a recomputed id.
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let empty = HashSet::new();
        mint_tutor_card(
            &mut store,
            "geo.md",
            "capital of spain?",
            &["Madrid".to_string()],
            100,
            &empty,
        )
        .unwrap();
        let err = mint_tutor_card(
            &mut store,
            "geo.md",
            "capital of spain?",
            &["Madrid".to_string()],
            200,
            &empty,
        )
        .unwrap_err();
        assert!(matches!(err, MintError::Duplicate));
    }

    #[test]
    fn mint_tutor_card_rejects_an_empty_side() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let err = mint_tutor_card(
            &mut store,
            "geo.md",
            "  ",
            &["Paris".to_string()],
            100,
            &HashSet::new(),
        )
        .unwrap_err();
        assert!(matches!(err, MintError::Malformed(_)));
    }

    #[test]
    fn mint_tutor_card_rejects_an_embedded_newline() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        // A newline inside a back element could smuggle an extra line or a `%` directive.
        let err = mint_tutor_card(
            &mut store,
            "geo.md",
            "capital?",
            &["Paris\n% direction: reverse".to_string()],
            100,
            &HashSet::new(),
        )
        .unwrap_err();
        assert!(matches!(err, MintError::Malformed(_)));
    }

    #[test]
    fn split_card_blocks_one_block_per_top_depth_front() {
        // Two plain cards separated by a blank line, so two blocks.
        let blocks = split_card_blocks("## a\n1\n\n## b\n2\n");
        assert_eq!(2, blocks.len());
        assert!(blocks[0].starts_with("## a"));
        assert!(blocks[1].starts_with("## b"));
    }

    #[test]
    fn split_card_blocks_keeps_indented_hash_and_directives_inside_a_block() {
        // A `#`-leading answer line, a per-card directive, and a `#?`-looking
        // indented line all stay inside the one card block.
        let text = "## front <!-- reveal: line -->\n#[derive(Clone)]\n#? not a front\n";
        let blocks = split_card_blocks(text);
        assert_eq!(1, blocks.len());
        assert!(blocks[0].contains("<!-- reveal: line -->"));
        assert!(blocks[0].contains("#[derive(Clone)]"));
        assert!(blocks[0].contains("#? not a front"));
    }

    #[test]
    fn split_card_blocks_is_one_block_for_a_cloze_and_drops_preamble() {
        // A leading deck-level directive/blank line is preamble (dropped); the
        // whole cloze block (front + `% reveal: cloze` + answer) is one block.
        let text =
            "---\nsource: x\n---\n\n## Complete the quote\nTo \\cloze{be} or not to \\cloze{be}\n";
        let blocks = split_card_blocks(text);
        assert_eq!(1, blocks.len());
        assert!(blocks[0].starts_with("## Complete the quote"));
        assert!(blocks[0].contains("\\cloze{be}"));
        assert!(!blocks[0].contains("% source:"));
    }

    /// `store_remediation_cards` with no deck-id dedup baseline (the common
    /// test case, the deck has no card equal to the remediation output).
    fn store_remediation(
        store: &mut Store,
        subject: &str,
        cards_text: &str,
        now_ms: u64,
        retire_after_days: Option<u32>,
    ) -> AnyResult<usize> {
        store_remediation_cards(
            store,
            subject,
            &std::collections::HashSet::new(),
            cards_text,
            now_ms,
            retire_after_days,
        )
    }

    #[test]
    fn failing_the_same_exam_twice_yields_zero_duplicate_gap_cards() {
        // Remediation dedups by canonical content (§7): re-running the same gap
        // text (a fresh random token each time) is recognized as already stored,
        // so nothing new is created the second time.
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let text = "## Why does X happen?\nbecause of Y\n";

        let first = store_remediation(&mut store, "d.md", text, 1_000, None).unwrap();
        assert_eq!(1, first, "the first failure creates the gap card");
        let second = store_remediation(&mut store, "d.md", text, 2_000, None).unwrap();
        assert_eq!(
            0, second,
            "the same gap again is a content dupe, not a new card"
        );
        assert_eq!(1, store.virtual_cards_for("d.md").len());
    }

    #[test]
    fn distinct_answer_cloze_holes_stay_distinct() {
        // Both holes hold the same token ("be"), so the sub-cards share the
        // sentence, but their ids differ by the `#cloze:k` hole index, so the
        // substrate keeps BOTH (no discriminator, no merge). Two sidecar
        // entries share one `text` but are keyed by distinct ids.
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let text = "## Complete the quote\nTo \\cloze{be} or not to \\cloze{be}\n";

        let n = store_remediation(&mut store, "d.md", text, 1_000, None).unwrap();
        assert_eq!(2, n, "both cloze sub-cards should be created, not deduped");
        let virtuals = store.virtual_cards_for("d.md");
        assert_eq!(2, virtuals.len());
        assert_ne!(
            virtuals[0].id, virtuals[1].id,
            "distinct ids for the two holes"
        );
        // They share the same stored block text.
        assert_eq!(virtuals[0].text, virtuals[1].text);
    }

    #[test]
    fn a_retired_multi_hole_block_revives_every_hole() {
        // A multi-hole cloze block dedups and revives as ONE unit: when every
        // hole's schedule has retired, re-running the same gap resets EVERY hole,
        // not just hole 0, because all sub-cards carry the block fingerprint.
        // (Mutation guard: fingerprinting each hole's hidden text instead lets
        // only hole 0 match, so hole 1 stays retired and this fails.)
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let text = "## Complete the quote\nTo \\cloze{be} or not to \\cloze{bee}\n";
        let cap = Some(30u32);

        let created = store_remediation(&mut store, "d.md", text, 1_000, cap).unwrap();
        assert_eq!(2, created, "both holes created on the first failure");
        let ids: Vec<String> = store
            .virtual_cards_for("d.md")
            .iter()
            .map(|vc| vc.id.clone())
            .collect();
        assert_eq!(2, ids.len());

        // Retire BOTH holes: give each a Recall schedule at/over the cap.
        for id in &ids {
            store.get_or_insert(id, 1_000).recall = Some(FsrsState {
                scheduled_days: 90,
                ..Default::default()
            });
        }
        for id in &ids {
            assert!(
                crate::session::is_retired_id(id, &store, cap),
                "precondition: both holes retired"
            );
        }

        // Re-run the same gap: every retired hole revives (schedule reset).
        let revived = store_remediation(&mut store, "d.md", text, 2_000, cap).unwrap();
        assert_eq!(2, revived, "every retired hole revives, not just hole 0");
        for id in &ids {
            assert!(
                !crate::session::is_retired_id(id, &store, cap),
                "revived, no longer retired"
            );
            assert_eq!(
                &CardState::new(2_000),
                store.get(id).unwrap(),
                "the hole's schedule was reset"
            );
        }
        // No new sidecar entries were minted.
        assert_eq!(2, store.virtual_cards_for("d.md").len());
    }

    #[test]
    fn a_plain_card_matching_a_holes_hidden_text_does_not_suppress_remediation() {
        // A plain deck card whose answer equals a cloze hole's hidden text must
        // NOT suppress a cloze remediation block: the block fingerprint includes
        // the literal `\cloze{...}` markers, which the plain answer lacks, so the
        // two differ. (Mutation guard: fingerprinting the hole's hidden text makes
        // them collide, the block is skipped, and this fails.)
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        // The deck already drills a PLAIN card whose answer is a bare "be".
        let plain =
            crate::l1::parse_str("d.md", "## Complete the quote <!-- id: p1 -->\nbe\n").unwrap();
        let deck_fingerprints: std::collections::HashSet<u64> =
            plain.iter().map(|c| c.content_fingerprint).collect();

        // Remediation emits a CLOZE block hiding "be" under the same heading.
        let cloze = "## Complete the quote\nTo \\cloze{be} or not to \\cloze{bee}\n";
        let created =
            store_remediation_cards(&mut store, "d.md", &deck_fingerprints, cloze, 1_000, None)
                .unwrap();
        assert_eq!(
            2, created,
            "the plain card must not suppress the cloze block"
        );
        assert_eq!(2, store.virtual_cards_for("d.md").len());
    }

    #[test]
    fn virtual_id_agrees_across_create_synth_and_promote() {
        // The load-bearing invariant: the id derived at CREATE
        // (`store_remediation_cards`), at SYNTH (`parse(parent, text).find`) and
        // at PROMOTE (append the block, re-parse the whole deck) must all agree,
        // for a plain card AND for every hole of a cloze card. Subject is always
        // `vc.parent`.
        for text in [
            "## Why does X?\npoint one\n",
            "## Complete the quote\nTo \\cloze{be} or not to \\cloze{bee}\n",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let deck_path = dir.path().join("d.md");
            std::fs::write(&deck_path, "## existing <!-- id: ex1 -->\nanswer\n").unwrap();
            let mut store = Store::open(dir.path().join("p.json")).unwrap();

            let created = store_remediation(&mut store, "d.md", text, 1_000, None).unwrap();
            let virtuals = store.virtual_cards_for("d.md");
            assert_eq!(created, virtuals.len());

            for vc in &virtuals {
                // SYNTH: re-derive from the stored text under the same parent.
                let synth = crate::l1::parse_str(&vc.parent, &vc.text)
                    .unwrap()
                    .into_iter()
                    .find(|c| c.id().as_deref() == Some(vc.id.as_str()))
                    .expect("synth reproduces the same id");
                assert_eq!(vc.id, synth.id().unwrap());
            }

            // PROMOTE one card: append its block to the deck, re-parse the whole
            // file, and confirm the matching card carries the same id.
            let vid = virtuals[0].id.clone();
            promote_virtual(&mut store, &vid, &deck_path).unwrap();
            let deck = crate::l1::parse_str("d.md", &std::fs::read_to_string(&deck_path).unwrap())
                .unwrap();
            assert!(
                deck.iter().any(|c| c.id().as_deref() == Some(vid.as_str())),
                "the appended deck card reproduces the id"
            );
        }
    }

    #[test]
    fn remediation_card_reveal_is_carried() {
        use crate::depth::Reveal;
        // A per-card `reveal:` directive survives in the stored `text` and re-parses on
        // synth.
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let text =
            "## Why does X? <!-- reveal: line -->\npoint one\n\n## fact card\nplain answer\n";

        store_remediation(&mut store, "d.md", text, 1_000, None).unwrap();
        // Synthesize each virtual card from its stored text to read its reveal.
        let synthesized: Vec<_> = store
            .virtual_cards_for("d.md")
            .iter()
            .map(|vc| {
                crate::l1::parse_str(&vc.parent, &vc.text)
                    .unwrap()
                    .remove(0)
            })
            .collect();
        let lined = synthesized
            .iter()
            .find(|c| c.front == "Why does X?")
            .unwrap();
        let plain = synthesized.iter().find(|c| c.front == "fact card").unwrap();
        assert_eq!(Some(Reveal::Line), lined.reveal);
        assert_eq!(None, plain.reveal);
    }
}
