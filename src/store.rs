use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result as AnyResult, bail};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{card::Card, deck, depth::Depth, scheduler::Grade};

const HISTORY_CAP: usize = 50;

// Pinned at 1 on purpose: pre-1.0 the shape changes freely and old data loads
// best-effort via `#[serde(default)]`, never gated on this field.
const CURRENT_VERSION: u32 = 1;

// Below this age a foreign write is ordinary roaming, not a live conflict.
pub const FOREIGN_WRITE_WARN_WINDOW_MS: u64 = 60 * 60 * 1000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Review {
    pub ts_ms: u64,
    #[serde(default = "default_review_grade")]
    pub grade: Grade,
    #[serde(default)]
    pub depth: Depth,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub propagated: bool,
}

// Defaults pre-grade history to Pass: history is cosmetic, so this can't corrupt scheduling.
fn default_review_grade() -> Grade {
    Grade::Pass
}

// Our own representation (all-u64 times), decoupled from rs-fsrs's `Card` so
// the store format doesn't depend on the crate's type.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FsrsState {
    pub stability: f64,
    pub difficulty: f64,
    pub reps: u32,
    pub lapses: u32,
    // rs-fsrs state: 0 New, 1 Learning, 2 Review, 3 Relearning (mirrors the crate's
    // discriminants).
    pub state: u8,
    pub scheduled_days: u32,
    pub last_review_ms: u64,
    pub due_ms: u64,
    // serde(default): a pre-existing store with no Goods yet reads as 0.
    #[serde(default)]
    pub learning_goods: u8,
}

impl FsrsState {
    // >= 2 also covers Relearning (3): a lapsed card still counts as graduated.
    pub fn graduated(&self) -> bool {
        self.state >= 2
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CardState {
    #[serde(default)]
    pub acquired_ms: u64,
    // Renamed from `fsrs` with no alias: a stored old `fsrs` key simply loads as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall: Option<FsrsState>,
    // Independent of `recall` on purpose: no cross-crediting between depths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconstruct: Option<FsrsState>,
    // Recognize is unscheduled: this flag, not an FsrsState, is its only progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognized_ms: Option<u64>,
    #[serde(default)]
    pub total_reviews: u32,
    #[serde(default)]
    pub total_passes: u32,
    #[serde(default)]
    pub streak: u32,
    // Capped to HISTORY_CAP; oldest entries drop first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Review>,
}

impl CardState {
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

    // Recognize is never scheduled: always answers None.
    pub fn schedule(&self, depth: Depth) -> Option<&FsrsState> {
        match depth {
            Depth::Recognize => None,
            Depth::Recall => self.recall.as_ref(),
            Depth::Reconstruct => self.reconstruct.as_ref(),
        }
    }

    // Recognize has no slot to hand back.
    pub fn schedule_slot(&mut self, depth: Depth) -> Option<&mut Option<FsrsState>> {
        match depth {
            Depth::Recognize => None,
            Depth::Recall => Some(&mut self.recall),
            Depth::Reconstruct => Some(&mut self.reconstruct),
        }
    }

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

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeckProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mastered_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exam_failed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_depth: Option<Depth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognized_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recalled_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconstructed_at_ms: Option<u64>,
}

// 21 days: the FSRS-community convention for a "mature" card.
pub const MATURE_STABILITY_DAYS: f64 = 21.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VirtualKind {
    Remediation,
    Tutor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VirtualCard {
    pub id: String,
    pub kind: VirtualKind,
    pub parent: String,
    pub text: String,
    pub created_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Writer {
    pub device: String,
    pub at_ms: u64,
}

// Store-internal, never card identity: freely bumpable; a stale version is
// ignored and rewritten, not mismatched.
pub const FP_VERSION: u8 = 1;

// Store-internal matcher data, not card identity: freely changeable.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HoleFingerprint {
    pub text_fp: u64,
    pub line_fp: u64,
}

// Keyed by the card's base token; store-internal, never part of card identity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CardRecords {
    // FP_VERSION at write time; a stale value is ignored and rewritten, not mismatched.
    pub version: u8,
    pub content_fp: u64,
    pub holes: Vec<HoleFingerprint>,
}

// Evicted here, not left on a live key, so a new word can never inherit an old schedule.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OrphanedHole {
    pub token: String,
    pub fp: HoleFingerprint,
    pub state: CardState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CascadeOutcome {
    pub remap: Vec<(u32, u32)>,
    pub orphaned: Vec<u32>,
    pub fresh: Vec<u32>,
}

pub fn realign_holes(stored: &[HoleFingerprint], file: &[HoleFingerprint]) -> CascadeOutcome {
    let mut consumed = vec![false; stored.len()];
    let mut matched: Vec<Option<usize>> = vec![None; file.len()];

    for (fi, fh) in file.iter().enumerate() {
        for (si, sh) in stored.iter().enumerate() {
            if !consumed[si] && sh.text_fp == fh.text_fp && sh.line_fp == fh.line_fp {
                consumed[si] = true;
                matched[fi] = Some(si);
                break;
            }
        }
    }
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

#[derive(Serialize, Deserialize)]
struct StoreFile {
    #[serde(default = "default_version")]
    version: u32,
    cards: HashMap<String, CardState>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    records: HashMap<String, CardRecords>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hole_orphans: Vec<OrphanedHole>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    decks: HashMap<String, DeckProgress>,
    // Raw JSON so a stale/old-shape entry can be dropped without failing the whole load.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    virtual_cards: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    writer: Option<Writer>,
}

pub struct Store {
    path: PathBuf,
    cards: HashMap<String, CardState>,
    decks: HashMap<String, DeckProgress>,
    virtual_cards: HashMap<String, VirtualCard>,
    records: HashMap<String, CardRecords>,
    hole_orphans: Vec<OrphanedHole>,
    // None leaves the existing on-disk writer marker untouched (tests/tools
    // don't masquerade as a device).
    pub device: Option<String>,
    last_writer: Option<Writer>,
}

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
        // A key of an unexpected charset is kept, not rejected: doctor material, not a load
        // failure.
        let cards = file.cards;
        // A stale/old-shape entry (pre-rework numeric id) is dropped, not a load
        // failure: regenerable content, not real progress.
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

    pub fn save(&self) -> Result<(), StoreError> {
        let io_err = |source| StoreError::Io {
            path: self.path.clone(),
            source,
        };

        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(io_err)?;
        }

        // Write side uses the real VirtualCard shape; only loading is lenient.
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
            // No device set: keep the existing marker instead of erasing it.
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

    pub fn foreign_writer(&self, my_device: &str, now_ms: u64) -> Option<(String, u64)> {
        let writer = self.last_writer.as_ref()?;
        if writer.device == my_device {
            return None;
        }
        Some((writer.device.clone(), now_ms.saturating_sub(writer.at_ms)))
    }

    pub fn recent_foreign_writer(&self, my_device: &str, now_ms: u64) -> Option<(String, u64)> {
        self.foreign_writer(my_device, now_ms)
            .filter(|(_, age_ms)| *age_ms < FOREIGN_WRITE_WARN_WINDOW_MS)
    }

    pub fn get(&self, card_id: &str) -> Option<&CardState> {
        self.cards.get(card_id)
    }

    // Reflects actual reviews, not merely opening the deck.
    pub fn last_review_ms(&self) -> Option<u64> {
        self.cards
            .values()
            .filter_map(|state| state.history.last().map(|review| review.ts_ms))
            .max()
    }

    pub fn get_or_insert(&mut self, card_id: &str, now_ms: u64) -> &mut CardState {
        self.cards
            .entry(card_id.to_string())
            .or_insert_with(|| CardState::new(now_ms))
    }

    pub fn remove(&mut self, card_id: &str) -> bool {
        self.cards.remove(card_id).is_some()
    }

    pub fn records(&self, token: &str) -> Option<&CardRecords> {
        self.records.get(token)
    }

    pub fn hole_orphans(&self) -> &[OrphanedHole] {
        &self.hole_orphans
    }

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
                    // Lower token wins a tie, for a deterministic pick.
                    if token < o.get() {
                        o.insert(token.clone());
                    }
                }
            }
        }
        best
    }

    // Does not run the hole cascade: callers must read old records via
    // realign_card_holes before this overwrites them.
    pub fn ensure_records(&mut self, card: &Card) {
        if let Some(token) = card.token.as_deref() {
            self.ensure_records_raw(token, card.content_fingerprint, &card.block_holes);
        }
    }

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

    pub fn realign_card_holes(
        &mut self,
        token: &str,
        file_holes: &[HoleFingerprint],
        content_fp: u64,
    ) -> Option<CascadeOutcome> {
        let outcome = match self.records.get(token) {
            Some(rec) if rec.version == FP_VERSION && rec.holes != file_holes => {
                let stored = rec.holes.clone();
                let outcome = realign_holes(&stored, file_holes);
                self.apply_hole_cascade(token, &stored, &outcome);
                Some(outcome)
            }
            _ => None,
        };
        self.ensure_records_raw(token, content_fp, file_holes);
        outcome
    }

    fn apply_hole_cascade(
        &mut self,
        token: &str,
        stored_holes: &[HoleFingerprint],
        outcome: &CascadeOutcome,
    ) {
        // Pulls every token-N entry, not just 0..stored_holes.len(): a stray
        // above the range must not survive to be inherited later.
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
        let mut leftover: Vec<u32> = old.keys().copied().collect();
        leftover.sort_unstable();
        for orphan in leftover {
            if let Some(state) = old.remove(&orphan) {
                // A stray (no stored record) carries a default fingerprint here.
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

    pub fn get_virtual(&self, id: &str) -> Option<&VirtualCard> {
        self.virtual_cards.get(id)
    }

    // Sidecar membership is the sole definition of "virtual"; the schedule
    // itself is an ordinary store.cards entry.
    pub fn is_virtual(&self, id: &str) -> bool {
        self.virtual_cards.contains_key(id)
    }

    pub fn insert_virtual(&mut self, card: VirtualCard) {
        self.virtual_cards.insert(card.id.clone(), card);
    }

    pub fn remove_virtual(&mut self, id: &str) -> bool {
        self.virtual_cards.remove(id).is_some()
    }

    // A cloze block shares one sidecar entry per hole; drop them ALL here or
    // promoting one hole orphans the rest with colliding ids.
    pub fn remove_virtual_block(&mut self, parent: &str, text: &str) -> usize {
        let before = self.virtual_cards.len();
        self.virtual_cards
            .retain(|_, vc| !(vc.parent == parent && vc.text == text));
        before - self.virtual_cards.len()
    }

    pub fn iter_virtual_cards(&self) -> impl Iterator<Item = &VirtualCard> {
        self.virtual_cards.values()
    }

    pub fn virtual_ids_with_content(&self, subject: &str, fingerprint: u64) -> Vec<String> {
        self.virtual_cards
            .values()
            .filter(|vc| vc.parent == subject && virtual_fingerprint(vc) == Some(fingerprint))
            .map(|vc| vc.id.clone())
            .collect()
    }

    pub fn virtual_cards_for(&self, subject: &str) -> Vec<&VirtualCard> {
        self.virtual_cards
            .values()
            .filter(|v| v.parent == subject)
            .collect()
    }

    pub fn deck_mastered(&self, subject: &str) -> bool {
        self.deck_mastered_at(subject).is_some()
    }

    pub fn deck_mastered_at(&self, subject: &str) -> Option<u64> {
        self.decks.get(subject).and_then(|d| d.mastered_at_ms)
    }

    // A pass clears any prior failed-exam cooldown.
    pub fn set_deck_mastered(&mut self, subject: &str, now_ms: u64) {
        let entry = self.decks.entry(subject.to_string()).or_default();
        entry.mastered_at_ms = Some(now_ms);
        entry.exam_failed_at_ms = None;
    }

    pub fn exam_failed_at(&self, subject: &str) -> Option<u64> {
        self.decks.get(subject).and_then(|d| d.exam_failed_at_ms)
    }

    pub fn set_exam_failed(&mut self, subject: &str, now_ms: u64) {
        self.decks
            .entry(subject.to_string())
            .or_default()
            .exam_failed_at_ms = Some(now_ms);
    }

    pub fn clear_deck_mastered(&mut self, subject: &str) -> bool {
        self.decks.remove(subject).is_some()
    }

    pub fn last_depth(&self, subject: &str) -> Option<Depth> {
        self.decks.get(subject).and_then(|d| d.last_depth)
    }

    pub fn set_last_depth(&mut self, subject: &str, depth: Depth) {
        self.decks
            .entry(subject.to_string())
            .or_default()
            .last_depth = Some(depth);
    }

    pub fn badge_earned(&self, subject: &str, depth: Depth) -> Option<u64> {
        let deck = self.decks.get(subject)?;
        match depth {
            Depth::Recognize => deck.recognized_at_ms,
            Depth::Recall => deck.recalled_at_ms,
            Depth::Reconstruct => deck.reconstructed_at_ms,
        }
    }

    // Also drops virtual cards: a reset must not leave them behind to keep drilling.
    pub fn clear(&mut self) -> usize {
        let n = self.cards.len();
        self.cards.clear();
        self.decks.clear();
        self.virtual_cards.clear();
        self.records.clear();
        self.hole_orphans.clear();
        n
    }

    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    pub fn virtual_len(&self) -> usize {
        self.virtual_cards.len()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    // A virtual card's own schedule key is never an orphan: it's a legitimate
    // local card with no deck file.
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
        // A pruned token's now-scheduleless records/shelf entries are dead weight and
        // drop too (uncounted); a still-live token's shelf entry stays as reclaim evidence.
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

    // Wipes every family the deck owned at once, so deliberate destruction
    // leaves no orphan.
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

// Never auto-pruned: cleared only by an explicit `alix reset --orphans`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Orphans {
    pub cards: Vec<String>,
    pub decks: Vec<String>,
}

impl Orphans {
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty() && self.decks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.cards.len() + self.decks.len()
    }
}

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

#[derive(Debug, thiserror::Error)]
pub enum MintError {
    #[error("the drafted card is malformed: {0}")]
    Malformed(String),
    #[error("a card with this content already exists in the deck")]
    Duplicate,
    #[error("cannot mint an identity token: {0}")]
    Mint(String),
}

// Dedup is by content, not id: every mint gets a fresh random token, so
// identical content would otherwise mint a duplicate.
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
    // Records must exist before the schedule entry: keep this order.
    store.ensure_records(card);
    store.get_or_insert(&id, now_ms);
    Ok(id)
}

pub fn badge_solid(cards: &[Card], store: &Store, depth: Depth) -> bool {
    // An empty deck is never solid (not vacuously true).
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

// High-water: an already-earned date survives a later drop below the mature line.
// Badges gate nothing here, bookkeeping only, never a lifecycle interaction.
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

/// No schedule transfer needed: the id was unified at mint time. Appends
/// before removing the sidecar, so a crash duplicates, never loses, the card.
pub fn promote_virtual(store: &mut Store, id: &str, deck_path: &Path) -> AnyResult<()> {
    let Some(vc) = store.get_virtual(id) else {
        bail!("no virtual card with id {id} to promote");
    };
    let text = vc.text.clone();
    let parent = vc.parent.clone();

    deck::append_cards(deck_path, &text)
        .with_context(|| format!("appending the promoted card to {}", deck_path.display()))?;

    store.remove_virtual_block(&parent, &text);
    store
        .save()
        .context("saving the store after promoting a virtual card")?;
    Ok(())
}

// Preamble before the first `## ` front (frontmatter, prose) is dropped: it belongs to no card.
pub fn split_card_blocks(text: &str) -> Vec<String> {
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    // Tracks fences so a `## ` line inside a code fence doesn't start a bogus block.
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

/// The deck file is never touched. Dedup is by content, not id: each block
/// gets a fresh random token, so a rerun must match by canonical content.
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
        // The token rides the `## ` line so the stored text re-parses to the same id forever.
        let token =
            crate::token::mint().map_err(|e| anyhow::anyhow!("cannot mint a token: {e}"))?;
        let block = stamp_block(block, &token);
        // A malformed block is a hard error, not a silently-dropped card.
        let cards = crate::l1::parse_str(subject, &block)?;
        let Some(first) = cards.first() else {
            continue;
        };
        // Fingerprint includes the literal `\cloze{}` markers, so a plain card
        // repeating a hole's hidden text can't collide with it.
        let fingerprint = first.content_fingerprint;
        if deck_fingerprints.contains(&fingerprint) {
            continue;
        }
        let existing = store.virtual_ids_with_content(subject, fingerprint);
        if existing.is_empty() {
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
                // Records must exist before the schedule entry: keep this order.
                store.ensure_records(card);
                store.get_or_insert(&id, now_ms);
                created_or_revived += 1;
            }
        } else if existing
            .iter()
            .all(|id| crate::session::is_retired_id(id, store, retire_after_days))
        {
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

fn virtual_fingerprint(vc: &VirtualCard) -> Option<u64> {
    let cards = crate::l1::parse_str(&vc.parent, &vc.text).ok()?;
    let card = cards
        .iter()
        .find(|c| c.id().as_deref() == Some(vc.id.as_str()))?;
    // Every sub-card of a block carries the same block-level fingerprint.
    Some(card.content_fingerprint)
}

fn stamp_block(block: &str, token: &str) -> String {
    match block.split_once('\n') {
        Some((front, rest)) => format!("{front} <!-- id: {token} -->\n{rest}"),
        None => format!("{block} <!-- id: {token} -->"),
    }
}

fn default_version() -> u32 {
    1
}

pub fn default_store_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "alix").map(|dirs| dirs.data_dir().join("progress.json"))
}

// Syncthing's own naming convention for conflict copies: `<stem>.sync-conflict-*.<ext>`.
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

// Plaintext on purpose: rename a machine by editing the file directly.
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

pub fn device_label() -> Option<String> {
    let dirs = directories::ProjectDirs::from("", "", "alix")?;
    device_label_in(dirs.data_dir())
}

// A keyed hasher stands in for an RNG here: good enough for a device label, no new dependency.
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

    // Arbitrary ints stand in for distinct hidden-text/context hashes (equality is all that
    // matters).
    fn hf(word: u64, context: u64) -> HoleFingerprint {
        HoleFingerprint {
            text_fp: word,
            line_fp: context,
        }
    }

    #[test]
    fn inserting_a_hole_shifts_neighbors_without_losing_schedules() {
        let a = hf(1, 10);
        let b = hf(2, 20);
        let fresh_word = hf(9, 90);
        let outcome = realign_holes(&[a, b], &[fresh_word, a, b]);
        assert_eq!(vec![(0, 1), (1, 2)], outcome.remap);
        assert_eq!(vec![0], outcome.fresh);
        assert!(outcome.orphaned.is_empty());
    }

    #[test]
    fn deleting_a_hole_orphans_exactly_that_record() {
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
        let a = hf(1, 10);
        let b = hf(2, 20);
        let outcome = realign_holes(&[a, b], &[b, a]);
        assert_eq!(vec![(0, 1), (1, 0)], outcome.remap);
        assert!(outcome.orphaned.is_empty());
        assert!(outcome.fresh.is_empty());
    }

    #[test]
    fn a_context_rewrite_still_matches_by_text_alone() {
        let stored = hf(1, 10);
        let rewritten = hf(1, 99);
        let outcome = realign_holes(&[stored], &[rewritten]);
        assert_eq!(vec![(0, 0)], outcome.remap);
        assert!(outcome.orphaned.is_empty());
        assert!(outcome.fresh.is_empty());
    }

    #[test]
    fn identical_twins_pair_in_document_order_on_both_sides() {
        let twin = hf(5, 50);
        let outcome = realign_holes(&[twin, twin], &[twin, twin]);
        assert_eq!(vec![(0, 0), (1, 1)], outcome.remap);
        assert!(outcome.orphaned.is_empty());
        assert!(outcome.fresh.is_empty());
    }

    #[test]
    fn word_and_context_both_changed_is_a_fresh_hole() {
        let stored = hf(1, 10);
        let changed = hf(7, 70);
        let outcome = realign_holes(&[stored], &[changed]);
        assert!(outcome.remap.is_empty());
        assert_eq!(vec![0], outcome.fresh);
        assert_eq!(vec![0], outcome.orphaned);
    }

    #[test]
    fn a_fresh_hole_always_wins_the_live_key_and_the_orphan_goes_to_the_shelf() {
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
        store.get_or_insert("tok-0", 0).total_reviews = 1;
        store.get_or_insert("tok-1", 0).total_reviews = 2;

        let z = hf(8, 80);
        let outcome = store.realign_card_holes(token, &[z, b], 100).unwrap();
        assert_eq!(vec![(1, 1)], outcome.remap);
        assert_eq!(vec![0], outcome.orphaned);
        assert_eq!(vec![0], outcome.fresh);

        assert_eq!(2, store.get("tok-1").unwrap().total_reviews);
        assert!(store.get("tok-0").is_none());
        assert_eq!(1, store.hole_orphans().len());
        assert_eq!("tok", store.hole_orphans()[0].token);
        assert_eq!(a, store.hole_orphans()[0].fp);
        assert_eq!(1, store.hole_orphans()[0].state.total_reviews);
        assert_eq!(vec![z, b], store.records(token).unwrap().holes);
    }

    #[test]
    fn a_stray_high_index_hole_entry_is_pulled_by_the_cascade_not_left_to_squat() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let token = "tok";
        let a = hf(1, 10);
        let b = hf(2, 20);
        store.ensure_records_raw(token, 100, &[a, b]);
        store.get_or_insert("tok-0", 0).total_reviews = 1;
        store.get_or_insert("tok-1", 0).total_reviews = 2;
        store.get_or_insert("tok-5", 0).total_reviews = 9;

        let outcome = store.realign_card_holes(token, &[b, a], 100).unwrap();
        assert_eq!(vec![(0, 1), (1, 0)], outcome.remap);

        assert_eq!(2, store.get("tok-0").unwrap().total_reviews, "b -> hole 0");
        assert_eq!(1, store.get("tok-1").unwrap().total_reviews, "a -> hole 1");
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
        store.records.insert(
            token.to_string(),
            CardRecords {
                version: FP_VERSION.wrapping_add(1),
                content_fp: 100,
                holes: vec![a],
            },
        );
        store.get_or_insert("tok-0", 0).total_reviews = 7;

        let outcome = store.realign_card_holes(token, &[a, b], 100);
        assert!(outcome.is_none());
        assert_eq!(7, store.get("tok-0").unwrap().total_reviews);
        assert!(store.hole_orphans().is_empty());
        let rec = store.records(token).unwrap();
        assert_eq!(FP_VERSION, rec.version);
        assert_eq!(vec![a, b], rec.holes);
    }

    #[test]
    fn orphans_are_the_keys_with_no_live_card_or_deck_and_prune_clears_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert("live", 0);
        store.get_or_insert("gone", 0);
        store.insert_virtual(VirtualCard {
            id: "vq".to_string(),
            kind: VirtualKind::Remediation,
            parent: "rust.md".to_string(),
            text: "## v <!-- id: vq -->\nb\n".to_string(),
            created_ms: 0,
        });
        store.get_or_insert("vq", 0);
        store.set_last_depth("rust.md", Depth::Recall);
        store.set_last_depth("deleted.md", Depth::Recall);

        let known_cards: HashSet<String> = ["live".to_string()].into_iter().collect();
        let known_subjects: HashSet<String> = ["rust.md".to_string()].into_iter().collect();
        let orphans = store.orphans(&known_cards, &known_subjects);
        assert_eq!(vec!["gone".to_string()], orphans.cards);
        assert_eq!(vec!["deleted.md".to_string()], orphans.decks);
        assert_eq!(2, orphans.len());

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

        store.get_or_insert("gonetoken", 0).total_reviews = 3;
        store.ensure_records_raw("gonetoken", 100, &[a]);
        store.hole_orphans.push(OrphanedHole {
            token: "gonetoken".to_string(),
            fp: a,
            state: CardState::new(0),
        });
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

        assert!(store.get("gonetoken").is_none());
        assert!(store.records("gonetoken").is_none());
        assert!(store.hole_orphans().iter().all(|o| o.token != "gonetoken"));
        assert!(store.get("livetoken").is_some());
        assert!(store.records("livetoken").is_some());
        assert!(store.hole_orphans().iter().any(|o| o.token == "livetoken"));
    }

    #[test]
    fn wipe_deck_clears_every_family_for_its_tokens_and_spares_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let a = hf(1, 10);

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

        assert_eq!(2, wiped, "the base and the hole schedule both count");
        assert!(store.get("doom").is_none());
        assert!(store.get("doom-0").is_none());
        assert!(store.records("doom").is_none());
        assert!(store.hole_orphans().iter().all(|o| o.token != "doom"));
        assert!(!store.deck_mastered("doomed.md"));
        assert!(store.get_virtual("vdoom").is_none());
        assert!(store.get("vdoom").is_none());
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

        let mut reloaded = Store::open(&path).unwrap();
        assert!(reloaded.deck_mastered("rust.md"));
        assert_eq!(Some(1234), reloaded.deck_mastered_at("rust.md"));
        assert!(reloaded.clear_deck_mastered("rust.md"));
        assert!(!reloaded.deck_mastered("rust.md"));
        assert!(!reloaded.clear_deck_mastered("rust.md"));
    }

    #[test]
    fn exam_failed_records_and_a_pass_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");

        let mut store = Store::open(&path).unwrap();
        assert_eq!(None, store.exam_failed_at("t.md"));
        store.set_exam_failed("t.md", 5000);
        assert_eq!(Some(5000), store.exam_failed_at("t.md"));
        assert!(!store.deck_mastered("t.md"));
        store.save().unwrap();

        let mut reloaded = Store::open(&path).unwrap();
        assert_eq!(Some(5000), reloaded.exam_failed_at("t.md"));
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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(&path, "{\"version\":1,\"cards\":{}}").unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.is_empty());
        assert!(!store.deck_mastered("anything.md"));
    }

    #[test]
    fn loads_a_store_file_without_a_version_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(&path, "{\"cards\":{}}").unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.is_empty());
    }

    #[test]
    fn loads_any_version_and_defaults_pre_grade_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(
            &path,
            r#"{"version":999,"cards":{"5":{"acquired_ms":7,"history":[{"ts_ms":100,"passed":false}]}}}"#,
        )
        .unwrap();
        let store = Store::open(&path).unwrap();
        let state = store.get("5").unwrap();
        assert_eq!(7, state.acquired_ms);
        assert_eq!(100, state.history[0].ts_ms);
        assert_eq!(Grade::Pass, state.history[0].grade);
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
        assert_eq!(0, store.clear());
    }

    const BORROW_TEXT: &str = "## What does the borrow checker enforce? <!-- id: vb1 -->\nExactly one mutable borrow, or many shared ones\n";

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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        std::fs::write(&path, "{\"version\":1,\"cards\":{}}").unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.is_empty());
        assert!(store.get_virtual("123").is_none());
    }

    #[test]
    fn an_old_shape_virtual_cards_object_loads_leniently_dropping_stale_entries() {
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
        assert_eq!(7, store.get("5").unwrap().acquired_ms);
        assert!(store.deck_mastered("rust.md"));
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
        assert_eq!(3, after.len());
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
        assert!(!store_path.exists());
    }

    #[test]
    fn promoting_one_hole_of_a_multi_hole_cloze_removes_every_holes_sidecar_entry() {
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
            store
                .get_or_insert(&id, 1000)
                .record_review(1000, Grade::Pass, Depth::Recall, false);
        }

        promote_virtual(&mut store, &id0, &deck_path).unwrap();

        assert!(store.get_virtual(&id0).is_none());
        assert!(store.get_virtual(&id1).is_none());
        assert!(store.get(&id0).is_some());
        assert!(store.get(&id1).is_some());

        let deck_text = std::fs::read_to_string(&deck_path).unwrap();
        let deck_cards = crate::l1::parse_str("rust.md", &deck_text).unwrap();
        assert_eq!(3, deck_cards.len());

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
        assert_eq!(1, state.total_passes);
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
        let json = r#"{"acquired_ms":1234,"fsrs":null}"#;
        let s: CardState = serde_json::from_str(json).unwrap();
        assert_eq!(s.acquired_ms, 1234);
        assert!(s.recall.is_none());
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

        store.get_or_insert(&cards[0].id().unwrap(), 0).recall = Some(FsrsState {
            stability: 3.0,
            ..Default::default()
        });

        assert!(!badge_solid(&cards, &store, Depth::Recall));
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
    }

    #[test]
    fn records_exist_whenever_an_entry_is_created() {
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
        let blocks = split_card_blocks("## a\n1\n\n## b\n2\n");
        assert_eq!(2, blocks.len());
        assert!(blocks[0].starts_with("## a"));
        assert!(blocks[1].starts_with("## b"));
    }

    #[test]
    fn split_card_blocks_keeps_indented_hash_and_directives_inside_a_block() {
        let text = "## front <!-- reveal: line -->\n#[derive(Clone)]\n#? not a front\n";
        let blocks = split_card_blocks(text);
        assert_eq!(1, blocks.len());
        assert!(blocks[0].contains("<!-- reveal: line -->"));
        assert!(blocks[0].contains("#[derive(Clone)]"));
        assert!(blocks[0].contains("#? not a front"));
    }

    #[test]
    fn split_card_blocks_is_one_block_for_a_cloze_and_drops_preamble() {
        let text =
            "---\nsource: x\n---\n\n## Complete the quote\nTo \\cloze{be} or not to \\cloze{be}\n";
        let blocks = split_card_blocks(text);
        assert_eq!(1, blocks.len());
        assert!(blocks[0].starts_with("## Complete the quote"));
        assert!(blocks[0].contains("\\cloze{be}"));
        assert!(!blocks[0].contains("% source:"));
    }

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
        assert_eq!(virtuals[0].text, virtuals[1].text);
    }

    #[test]
    fn a_retired_multi_hole_block_revives_every_hole() {
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
        assert_eq!(2, store.virtual_cards_for("d.md").len());
    }

    #[test]
    fn a_plain_card_matching_a_holes_hidden_text_does_not_suppress_remediation() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let plain =
            crate::l1::parse_str("d.md", "## Complete the quote <!-- id: p1 -->\nbe\n").unwrap();
        let deck_fingerprints: std::collections::HashSet<u64> =
            plain.iter().map(|c| c.content_fingerprint).collect();

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
                let synth = crate::l1::parse_str(&vc.parent, &vc.text)
                    .unwrap()
                    .into_iter()
                    .find(|c| c.id().as_deref() == Some(vc.id.as_str()))
                    .expect("synth reproduces the same id");
                assert_eq!(vc.id, synth.id().unwrap());
            }

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
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let text =
            "## Why does X? <!-- reveal: line -->\npoint one\n\n## fact card\nplain answer\n";

        store_remediation(&mut store, "d.md", text, 1_000, None).unwrap();
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
