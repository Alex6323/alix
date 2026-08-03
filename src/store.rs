use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result as AnyResult, bail};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    card::Card,
    deck::{self, Deck},
    depth::Depth,
    scheduler::Grade,
};

const HISTORY_CAP: usize = 50;

const DECK_DOCUMENT_VERSION: u32 = 1;

// Below this age a foreign write is ordinary roaming, not a live conflict.
pub const FOREIGN_WRITE_WARN_WINDOW_MS: u64 = 60 * 60 * 1000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub ts_ms: u64,
    pub grade: Grade,
    pub depth: Depth,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub propagated: bool,
}

// Our own representation (all-u64 times), decoupled from rs-fsrs's `Card` so
// the store format doesn't depend on the crate's type.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
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
    pub learning_goods: u8,
}

impl FsrsState {
    // >= 2 also covers Relearning (3): a lapsed card still counts as graduated.
    pub fn graduated(&self) -> bool {
        self.state >= 2
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CardState {
    // None: presented but never acquired (an entry can exist from
    // presentation alone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acquired_ms: Option<u64>,
    // First time the card was the displayed card in any session; correctness
    // plays no part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presented_ms: Option<u64>,
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
            acquired_ms: Some(now_ms),
            presented_ms: None,
            recall: None,
            reconstruct: None,
            recognized_ms: None,
            total_reviews: 0,
            total_passes: 0,
            streak: 0,
            history: Vec::new(),
        }
    }

    // Presentation alone sets none of these: any of them means the learner
    // did something with the card beyond being shown it.
    pub fn engaged(&self) -> bool {
        self.acquired_ms.is_some()
            || self.recognized_ms.is_some()
            || self.recall.is_some()
            || self.reconstruct.is_some()
            || self.total_reviews > 0
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
#[serde(deny_unknown_fields)]
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

impl DeckProgress {
    pub(crate) fn is_empty(&self) -> bool {
        *self == DeckProgress::default()
    }
}

// 21 days: the FSRS-community convention for a "mature" card.
pub const MATURE_STABILITY_DAYS: f64 = 21.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VirtualKind {
    Remediation,
    Tutor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VirtualCard {
    pub id: String,
    pub kind: VirtualKind,
    pub deck: String,
    pub text: String,
    pub created_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Writer {
    pub device: String,
    pub at_ms: u64,
}

// Store-internal, never card identity: freely bumpable; a stale version is
// ignored and rewritten, not mismatched.
pub const FP_VERSION: u8 = 2;

// Store-internal matcher data, not card identity: freely changeable.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HoleFingerprint {
    pub text_fp: u64,
    pub line_fp: u64,
}

// Keyed by the card's base token; store-internal, never part of card identity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CardRecords {
    // FP_VERSION at write time; a stale value is ignored and rewritten, not mismatched.
    pub version: u8,
    pub holes: Vec<HoleFingerprint>,
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
#[serde(deny_unknown_fields)]
struct DeckStoreFile {
    version: u32,
    deck_id: String,
    // Display only, never a key.
    subject: String,
    revision: u64,
    cards: HashMap<String, CardState>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    records: HashMap<String, CardRecords>,
    #[serde(default, skip_serializing_if = "DeckProgress::is_empty")]
    deck: DeckProgress,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    virtual_cards: HashMap<String, VirtualCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    writer: Option<Writer>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StoreDocumentData {
    pub cards: HashMap<String, CardState>,
    pub records: HashMap<String, CardRecords>,
    pub deck: DeckProgress,
    pub virtual_cards: HashMap<String, VirtualCard>,
    pub writer: Option<Writer>,
}

enum StoreBacking {
    Deck {
        deck_id: String,
        subject: String,
        revision: AtomicU64,
    },
    Aggregate {
        documents: Mutex<Vec<StoreDocument>>,
        owners: StoreOwners,
    },
}

// A snapshot clone for immutable read projections (catalog row status): the
// atomics and the document mutex make the derive impossible, so this copies
// their current values.
impl Clone for StoreBacking {
    fn clone(&self) -> Self {
        match self {
            StoreBacking::Deck {
                deck_id,
                subject,
                revision,
            } => StoreBacking::Deck {
                deck_id: deck_id.clone(),
                subject: subject.clone(),
                revision: AtomicU64::new(revision.load(Ordering::Relaxed)),
            },
            StoreBacking::Aggregate { documents, owners } => StoreBacking::Aggregate {
                documents: Mutex::new(
                    documents
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .clone(),
                ),
                owners: owners.clone(),
            },
        }
    }
}

#[derive(Clone)]
struct StoreDocument {
    path: PathBuf,
    deck_id: String,
    subject: String,
    revision: u64,
    original: StoreDocumentData,
}

#[derive(Clone, Default)]
struct StoreOwners {
    cards: HashMap<String, String>,
    records: HashMap<String, String>,
    decks: HashMap<String, String>,
    virtual_cards: HashMap<String, String>,
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    cards: HashMap<String, CardState>,
    decks: HashMap<String, DeckProgress>,
    virtual_cards: HashMap<String, VirtualCard>,
    records: HashMap<String, CardRecords>,
    // None leaves the existing on-disk writer marker untouched (tests/tools
    // don't masquerade as a device).
    pub device: Option<String>,
    last_writer: Option<Writer>,
    backing: StoreBacking,
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
    #[error("{path}: unsupported progress document version {version}")]
    Version { path: PathBuf, version: u32 },
    #[error("{path}: progress document belongs to deck `{actual}`, expected `{expected}`")]
    DeckOwner {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("{path}: stale progress revision {loaded}; disk is at {disk}")]
    StaleRevision {
        path: PathBuf,
        loaded: u64,
        disk: u64,
    },
    #[error("duplicate {kind} key `{key}` across per-deck progress documents")]
    DuplicateKey { kind: &'static str, key: String },
    #[error("cannot save aggregate progress: {kind} key `{key}` has no owning deck")]
    UnownedKey { kind: &'static str, key: String },
    #[error("{subject}: deck is not initialized")]
    MissingDeckId { subject: String },
}

fn deck_revision(path: &Path, expected_deck_id: &str) -> Result<u64, StoreError> {
    if !path.exists() {
        return Ok(0);
    }
    let text = std::fs::read_to_string(path).map_err(|source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file: DeckStoreFile = serde_json::from_str(&text).map_err(|source| StoreError::Format {
        path: path.to_path_buf(),
        source,
    })?;
    if file.version != DECK_DOCUMENT_VERSION {
        return Err(StoreError::Version {
            path: path.to_path_buf(),
            version: file.version,
        });
    }
    if file.deck_id != expected_deck_id {
        return Err(StoreError::DeckOwner {
            path: path.to_path_buf(),
            expected: expected_deck_id.to_string(),
            actual: file.deck_id,
        });
    }
    Ok(file.revision)
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), StoreError> {
    let io_err = |source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    };
    let json = serde_json::to_string_pretty(value).map_err(|source| StoreError::Format {
        path: path.to_path_buf(),
        source,
    })?;
    let tmp = path.with_extension("json.tmp");
    crate::fsio::replace_file(&tmp, path, json.as_bytes()).map_err(io_err)
}

pub(crate) fn write_deck_data(
    path: &Path,
    deck_id: &str,
    subject: &str,
    revision: u64,
    data: &StoreDocumentData,
) -> Result<(), StoreError> {
    if let Some(dir) = path.parent() {
        crate::fsio::create_dir_all(dir).map_err(|source| StoreError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let file = DeckStoreFile {
        version: DECK_DOCUMENT_VERSION,
        deck_id: deck_id.to_string(),
        subject: subject.to_string(),
        revision,
        cards: data.cards.clone(),
        records: data.records.clone(),
        deck: data.deck,
        virtual_cards: data.virtual_cards.clone(),
        writer: data.writer.clone(),
    };
    write_json_atomic(path, &file)
}

pub(crate) fn read_deck_data(
    path: &Path,
    expected_deck_id: &str,
    current_subject: Option<&str>,
) -> Result<(u64, String, StoreDocumentData), StoreError> {
    let text = std::fs::read_to_string(path).map_err(|source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file: DeckStoreFile = serde_json::from_str(&text).map_err(|source| StoreError::Format {
        path: path.to_path_buf(),
        source,
    })?;
    if file.version != DECK_DOCUMENT_VERSION {
        return Err(StoreError::Version {
            path: path.to_path_buf(),
            version: file.version,
        });
    }
    if file.deck_id != expected_deck_id {
        return Err(StoreError::DeckOwner {
            path: path.to_path_buf(),
            expected: expected_deck_id.to_string(),
            actual: file.deck_id,
        });
    }
    let subject = current_subject.unwrap_or(&file.subject).to_string();
    let mut virtual_cards = file.virtual_cards;
    for card in virtual_cards.values_mut() {
        card.deck.clear();
        card.deck.push_str(expected_deck_id);
    }
    Ok((
        file.revision,
        subject,
        StoreDocumentData {
            cards: file.cards,
            records: file.records,
            deck: file.deck,
            virtual_cards,
            writer: file.writer,
        },
    ))
}

fn merge_owned<T: Clone>(
    target: &mut HashMap<String, T>,
    owners: &mut HashMap<String, String>,
    source: &HashMap<String, T>,
    deck_id: &str,
    kind: &'static str,
) -> Result<(), StoreError> {
    for (key, value) in source {
        if target.contains_key(key) {
            return Err(StoreError::DuplicateKey {
                kind,
                key: key.clone(),
            });
        }
        target.insert(key.clone(), value.clone());
        owners.insert(key.clone(), deck_id.to_string());
    }
    Ok(())
}

fn reject_unowned<T>(
    values: &HashMap<String, T>,
    owners: &HashMap<String, String>,
    kind: &'static str,
) -> Result<(), StoreError> {
    match values.keys().find(|key| !owners.contains_key(*key)) {
        Some(key) => Err(StoreError::UnownedKey {
            kind,
            key: key.clone(),
        }),
        None => Ok(()),
    }
}

fn owned_values<T: Clone>(
    values: &HashMap<String, T>,
    owners: &HashMap<String, String>,
    deck_id: &str,
) -> HashMap<String, T> {
    values
        .iter()
        .filter(|(key, _)| owners.get(*key).is_some_and(|owner| owner == deck_id))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let deck_id = crate::state::deck_id_from_document(&path)
                .ok_or_else(|| StoreError::MissingDeckId {
                    subject: path.display().to_string(),
                })?
                .to_string();
            return Self::open_deck(&path, &deck_id, format!("{deck_id}.md"));
        }
        Self::open_aggregate_for(path, &[])
    }

    pub fn open_deck(
        path: impl AsRef<Path>,
        deck_id: impl Into<String>,
        subject: impl Into<String>,
    ) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let deck_id = deck_id.into();
        let subject = subject.into();
        if !path.exists() {
            return Ok(Self {
                path,
                cards: HashMap::new(),
                decks: HashMap::new(),
                virtual_cards: HashMap::new(),
                records: HashMap::new(),
                device: None,
                last_writer: None,
                backing: StoreBacking::Deck {
                    deck_id,
                    subject,
                    revision: AtomicU64::new(0),
                },
            });
        }

        let text = std::fs::read_to_string(&path).map_err(|source| StoreError::Io {
            path: path.clone(),
            source,
        })?;
        let file: DeckStoreFile =
            serde_json::from_str(&text).map_err(|source| StoreError::Format {
                path: path.clone(),
                source,
            })?;
        if file.version != DECK_DOCUMENT_VERSION {
            return Err(StoreError::Version {
                path,
                version: file.version,
            });
        }
        if file.deck_id != deck_id {
            return Err(StoreError::DeckOwner {
                path,
                expected: deck_id,
                actual: file.deck_id,
            });
        }

        let mut virtual_cards = file.virtual_cards;
        for card in virtual_cards.values_mut() {
            card.deck.clear();
            card.deck.push_str(&deck_id);
        }
        // Deck-level state is keyed by the stable id in memory, never by the
        // filename; the document holds exactly one deck's progress.
        let mut decks = HashMap::new();
        if !file.deck.is_empty() {
            decks.insert(deck_id.clone(), file.deck);
        }
        Ok(Self {
            path,
            cards: file.cards,
            decks,
            virtual_cards,
            records: file.records,
            device: None,
            last_writer: file.writer,
            backing: StoreBacking::Deck {
                deck_id,
                subject,
                revision: AtomicU64::new(file.revision),
            },
        })
    }

    pub fn open_for_decks(path: impl AsRef<Path>, decks: &[Deck]) -> Result<Self, StoreError> {
        Self::open_aggregate_for(path.as_ref().to_path_buf(), decks)
    }

    fn open_aggregate_for(path: PathBuf, expected_decks: &[Deck]) -> Result<Self, StoreError> {
        let mut document_paths: Vec<PathBuf> = if path.is_dir() {
            std::fs::read_dir(&path)
                .map_err(|source| StoreError::Io {
                    path: path.clone(),
                    source,
                })?
                .map(|entry| {
                    entry
                        .map(|entry| entry.path())
                        .map_err(|source| StoreError::Io {
                            path: path.clone(),
                            source,
                        })
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        document_paths.retain(|path| {
            path.is_file()
                && path.extension().is_some_and(|ext| ext == "json")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_none_or(|name| !crate::workspace::is_conflict_name(name))
        });
        document_paths.sort();

        let mut cards = HashMap::new();
        let mut decks = HashMap::new();
        let mut virtual_cards = HashMap::new();
        let mut records = HashMap::new();
        let mut owners = StoreOwners::default();
        let mut expected = HashMap::new();
        for deck in expected_decks {
            let deck_id = deck
                .deck_token
                .as_deref()
                .ok_or_else(|| StoreError::MissingDeckId {
                    subject: deck.subject.clone(),
                })?;
            expected.insert(deck_id.to_string(), deck.subject.clone());
            for card in &deck.cards {
                if let Some(card_id) = card.id() {
                    owners.cards.insert(card_id, deck_id.to_string());
                }
                if let Some(token) = card.token.as_deref() {
                    owners
                        .records
                        .insert(token.to_string(), deck_id.to_string());
                }
            }
            owners
                .decks
                .insert(deck_id.to_string(), deck_id.to_string());
        }
        let mut documents = Vec::new();
        let mut loaded_decks = HashSet::new();
        let mut last_writer: Option<Writer> = None;
        for document_path in document_paths {
            let Some(deck_id) =
                crate::state::deck_id_from_document(&document_path).map(str::to_string)
            else {
                continue;
            };
            let current_subject = expected.get(&deck_id).map(String::as_str);
            let (revision, subject, data) =
                read_deck_data(&document_path, &deck_id, current_subject)?;
            merge_owned(&mut cards, &mut owners.cards, &data.cards, &deck_id, "card")?;
            merge_owned(
                &mut records,
                &mut owners.records,
                &data.records,
                &deck_id,
                "record",
            )?;
            // Registered even for an empty entry, so a later `insert_virtual`
            // can still resolve this deck's ownership.
            owners.decks.insert(deck_id.clone(), deck_id.clone());
            if !data.deck.is_empty() {
                decks.insert(deck_id.clone(), data.deck);
            }
            merge_owned(
                &mut virtual_cards,
                &mut owners.virtual_cards,
                &data.virtual_cards,
                &deck_id,
                "virtual card",
            )?;
            if data.writer.as_ref().is_some_and(|candidate| {
                last_writer
                    .as_ref()
                    .is_none_or(|current| candidate.at_ms > current.at_ms)
            }) {
                last_writer.clone_from(&data.writer);
            }
            documents.push(StoreDocument {
                path: document_path,
                deck_id: deck_id.clone(),
                subject,
                revision,
                original: data,
            });
            loaded_decks.insert(deck_id);
        }
        for (deck_id, subject) in expected {
            if loaded_decks.contains(&deck_id) {
                continue;
            }
            documents.push(StoreDocument {
                path: path.join(format!("{deck_id}.json")),
                deck_id,
                subject,
                revision: 0,
                original: StoreDocumentData::default(),
            });
        }
        documents.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self {
            path,
            cards,
            decks,
            virtual_cards,
            records,
            device: None,
            last_writer,
            backing: StoreBacking::Aggregate {
                documents: Mutex::new(documents),
                owners,
            },
        })
    }

    pub fn save(&self) -> Result<(), StoreError> {
        match &self.backing {
            StoreBacking::Deck {
                deck_id,
                subject,
                revision,
            } => self.save_deck(deck_id, subject, revision),
            StoreBacking::Aggregate { documents, owners } => self.save_aggregate(documents, owners),
        }
    }

    fn save_deck(
        &self,
        deck_id: &str,
        subject: &str,
        revision: &AtomicU64,
    ) -> Result<(), StoreError> {
        let io_err = |source| StoreError::Io {
            path: self.path.clone(),
            source,
        };
        if let Some(dir) = self.path.parent() {
            crate::fsio::create_dir_all(dir).map_err(io_err)?;
        }
        let loaded = revision.load(Ordering::Relaxed);
        let disk = deck_revision(&self.path, deck_id)?;
        if disk != loaded {
            return Err(StoreError::StaleRevision {
                path: self.path.clone(),
                loaded,
                disk,
            });
        }
        let next = loaded.saturating_add(1);
        let file = DeckStoreFile {
            version: DECK_DOCUMENT_VERSION,
            deck_id: deck_id.to_string(),
            subject: subject.to_string(),
            revision: next,
            cards: self.cards.clone(),
            records: self.records.clone(),
            deck: self.decks.get(deck_id).cloned().unwrap_or_default(),
            virtual_cards: self.virtual_cards.clone(),
            writer: self.writer_for_save(),
        };
        write_json_atomic(&self.path, &file)?;
        revision.store(next, Ordering::Relaxed);
        Ok(())
    }

    fn writer_for_save(&self) -> Option<Writer> {
        self.device
            .clone()
            .map(|device| Writer {
                device,
                at_ms: crate::time::now_ms(),
            })
            .or_else(|| self.last_writer.clone())
    }

    fn save_aggregate(
        &self,
        documents: &Mutex<Vec<StoreDocument>>,
        owners: &StoreOwners,
    ) -> Result<(), StoreError> {
        reject_unowned(&self.cards, &owners.cards, "card")?;
        reject_unowned(&self.records, &owners.records, "record")?;
        reject_unowned(&self.decks, &owners.decks, "deck")?;
        reject_unowned(&self.virtual_cards, &owners.virtual_cards, "virtual card")?;

        let mut documents = documents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut changed = Vec::new();
        for (index, document) in documents.iter().enumerate() {
            let data = StoreDocumentData {
                cards: owned_values(&self.cards, &owners.cards, &document.deck_id),
                records: owned_values(&self.records, &owners.records, &document.deck_id),
                deck: self
                    .decks
                    .get(&document.deck_id)
                    .cloned()
                    .unwrap_or_default(),
                virtual_cards: owned_values(
                    &self.virtual_cards,
                    &owners.virtual_cards,
                    &document.deck_id,
                ),
                writer: document.original.writer.clone(),
            };
            if data != document.original {
                let disk = deck_revision(&document.path, &document.deck_id)?;
                if disk != document.revision {
                    return Err(StoreError::StaleRevision {
                        path: document.path.clone(),
                        loaded: document.revision,
                        disk,
                    });
                }
                changed.push((index, data));
            }
        }
        for (index, mut data) in changed {
            let document = &mut documents[index];
            data.writer = self.writer_for_save();
            let next = document.revision.saturating_add(1);
            write_deck_data(
                &document.path,
                &document.deck_id,
                &document.subject,
                next,
                &data,
            )?;
            document.revision = next;
            document.original = data;
        }
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

    // The entry's mere existence no longer implies engagement (presentation
    // creates one too): queue/on-ramp classification reads this instead of
    // `get`.
    pub fn progress(&self, card_id: &str) -> Option<&CardState> {
        self.cards.get(card_id).filter(|state| state.engaged())
    }

    // Once-only: the first call stamps and answers true; every later call is
    // a no-op. The default entry acquires nothing.
    pub fn note_presented(&mut self, card_id: &str, now_ms: u64) -> bool {
        let state = self.cards.entry(card_id.to_string()).or_default();
        if state.presented_ms.is_none() {
            state.presented_ms = Some(now_ms);
            true
        } else {
            false
        }
    }

    // A reveal is an encounter: seeing the answer engages the card even if
    // the session ends without the Seen acknowledgment. Once-only, and never
    // on a card the learner already engaged some other way.
    pub fn note_revealed(&mut self, card_id: &str, now_ms: u64) -> bool {
        let state = self.cards.entry(card_id.to_string()).or_default();
        if state.engaged() {
            false
        } else {
            state.acquired_ms = Some(now_ms);
            true
        }
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

    // Does not run the hole cascade: callers must read old records via
    // realign_card_holes before this overwrites them.
    pub fn ensure_records(&mut self, card: &Card) {
        if let Some(token) = card.token.as_deref() {
            self.ensure_records_raw(token, &card.block_holes);
        }
    }

    pub fn ensure_records_raw(&mut self, token: &str, holes: &[HoleFingerprint]) {
        self.records.insert(
            token.to_string(),
            CardRecords {
                version: FP_VERSION,
                holes: holes.to_vec(),
            },
        );
    }

    pub fn realign_card_holes(
        &mut self,
        token: &str,
        file_holes: &[HoleFingerprint],
    ) -> Option<CascadeOutcome> {
        let outcome = match self.records.get(token) {
            Some(rec) if rec.version == FP_VERSION && rec.holes != file_holes => {
                let stored = rec.holes.clone();
                let outcome = realign_holes(&stored, file_holes);
                self.apply_hole_cascade(token, &outcome);
                Some(outcome)
            }
            _ => None,
        };
        self.ensure_records_raw(token, file_holes);
        outcome
    }

    fn apply_hole_cascade(&mut self, token: &str, outcome: &CascadeOutcome) {
        let prefix = format!("{token}-");
        let hole_keys: Vec<(u32, String)> = self
            .cards
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .filter_map(|key| match crate::token::parse_prefixed_card_id(key) {
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
        // Re-add only remapped entries; a stray token-N schedule must not be inherited.
        for (from, to) in &outcome.remap {
            if let Some(state) = old.remove(from) {
                let key = crate::token::card_id(token, Some(*to), false);
                self.cards.insert(key, state);
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
        if let StoreBacking::Aggregate { owners, .. } = &mut self.backing
            && let Some(deck_id) = owners.decks.get(&card.deck).cloned()
        {
            owners.cards.insert(card.id.clone(), deck_id.clone());
            owners.virtual_cards.insert(card.id.clone(), deck_id);
        }
        self.virtual_cards.insert(card.id.clone(), card);
    }

    pub fn rebind_replaced_deck(
        &mut self,
        old_deck_id: &str,
        deck: &Deck,
    ) -> Result<(), StoreError> {
        let new_deck_id = deck
            .deck_token
            .as_deref()
            .ok_or_else(|| StoreError::MissingDeckId {
                subject: deck.subject.clone(),
            })?;
        match &mut self.backing {
            StoreBacking::Deck {
                deck_id,
                subject,
                revision,
            } => {
                let progress = self.path.parent().ok_or_else(|| StoreError::Io {
                    path: self.path.clone(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "progress document has no parent directory",
                    ),
                })?;
                self.path = progress.join(format!("{new_deck_id}.json"));
                deck_id.replace_range(.., new_deck_id);
                subject.clone_from(&deck.subject);
                revision.store(0, Ordering::Relaxed);
            }
            StoreBacking::Aggregate { documents, owners } => {
                let documents = documents
                    .get_mut()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                documents.retain(|document| document.deck_id != old_deck_id);
                documents.push(StoreDocument {
                    path: self.path.join(format!("{new_deck_id}.json")),
                    deck_id: new_deck_id.to_string(),
                    subject: deck.subject.clone(),
                    revision: 0,
                    original: StoreDocumentData::default(),
                });
                documents.sort_by(|left, right| left.path.cmp(&right.path));
                owners.cards.retain(|_, owner| owner != old_deck_id);
                owners.records.retain(|_, owner| owner != old_deck_id);
                owners.decks.retain(|_, owner| owner != old_deck_id);
                owners.virtual_cards.retain(|_, owner| owner != old_deck_id);
                owners
                    .decks
                    .insert(new_deck_id.to_string(), new_deck_id.to_string());
                for card in &deck.cards {
                    if let Some(card_id) = card.id() {
                        owners.cards.insert(card_id, new_deck_id.to_string());
                    }
                    if let Some(token) = card.token.as_deref() {
                        owners
                            .records
                            .insert(token.to_string(), new_deck_id.to_string());
                    }
                }
            }
        }
        Ok(())
    }

    pub fn remove_virtual(&mut self, id: &str) -> bool {
        self.virtual_cards.remove(id).is_some()
    }

    // A cloze block shares one sidecar entry per hole; drop them ALL here or
    // promoting one hole orphans the rest with colliding ids.
    pub fn remove_virtual_block(&mut self, deck_id: &str, text: &str) -> usize {
        let before = self.virtual_cards.len();
        self.virtual_cards
            .retain(|_, vc| !(vc.deck == deck_id && vc.text == text));
        before - self.virtual_cards.len()
    }

    pub fn iter_virtual_cards(&self) -> impl Iterator<Item = &VirtualCard> {
        self.virtual_cards.values()
    }

    pub fn virtual_ids_with_content(&self, deck_id: &str, fingerprint: u64) -> Vec<String> {
        self.virtual_cards
            .values()
            .filter(|vc| vc.deck == deck_id && virtual_fingerprint(vc) == Some(fingerprint))
            .map(|vc| vc.id.clone())
            .collect()
    }

    pub fn virtual_cards_for(&self, deck_id: &str) -> Vec<&VirtualCard> {
        self.virtual_cards
            .values()
            .filter(|v| v.deck == deck_id)
            .collect()
    }

    pub fn deck_mastered(&self, deck_id: &str) -> bool {
        self.deck_mastered_at(deck_id).is_some()
    }

    pub fn deck_mastered_at(&self, deck_id: &str) -> Option<u64> {
        self.decks.get(deck_id).and_then(|d| d.mastered_at_ms)
    }

    // A pass clears any prior failed-exam cooldown.
    pub fn set_deck_mastered(&mut self, deck_id: &str, now_ms: u64) {
        let entry = self.decks.entry(deck_id.to_string()).or_default();
        entry.mastered_at_ms = Some(now_ms);
        entry.exam_failed_at_ms = None;
    }

    pub fn exam_failed_at(&self, deck_id: &str) -> Option<u64> {
        self.decks.get(deck_id).and_then(|d| d.exam_failed_at_ms)
    }

    pub fn set_exam_failed(&mut self, deck_id: &str, now_ms: u64) {
        self.decks
            .entry(deck_id.to_string())
            .or_default()
            .exam_failed_at_ms = Some(now_ms);
    }

    pub fn clear_deck_mastered(&mut self, deck_id: &str) -> bool {
        self.decks.remove(deck_id).is_some()
    }

    pub fn last_depth(&self, deck_id: &str) -> Option<Depth> {
        self.decks.get(deck_id).and_then(|d| d.last_depth)
    }

    pub fn set_last_depth(&mut self, deck_id: &str, depth: Depth) {
        self.decks
            .entry(deck_id.to_string())
            .or_default()
            .last_depth = Some(depth);
    }

    pub fn badge_earned(&self, deck_id: &str, depth: Depth) -> Option<u64> {
        let deck = self.decks.get(deck_id)?;
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
        known_deck_ids: &HashSet<String>,
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
            .filter(|k| !known_deck_ids.contains(*k))
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
        for deck_id in &orphans.decks {
            if self.decks.remove(deck_id).is_some() {
                removed += 1;
            }
        }
        // A fully-pruned token's now-scheduleless records are dead weight and drop too (uncounted).
        let pruned_tokens: HashSet<&str> = orphans
            .cards
            .iter()
            .filter_map(|id| crate::token::parse_prefixed_card_id(id).map(|(token, _, _)| token))
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
        }
        removed
    }

    // Wipes every family the deck owned at once, so deliberate destruction
    // leaves no orphan.
    pub fn wipe_deck(&mut self, tokens: &HashSet<String>, deck_id: &str) -> usize {
        let doomed: Vec<String> = self
            .cards
            .keys()
            .filter(|id| {
                crate::token::parse_prefixed_card_id(id)
                    .is_some_and(|(token, _, _)| tokens.contains(token))
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
        self.decks.remove(deck_id);
        let virtuals: Vec<String> = self
            .virtual_cards
            .values()
            .filter(|vc| vc.deck == deck_id)
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
    deck_id: &str,
    cooldown_secs: u64,
    now_ms: u64,
) -> Option<u64> {
    if cooldown_secs == 0 {
        return None;
    }
    let until = store
        .exam_failed_at(deck_id)?
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
    deck_id: &str,
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
    let token = crate::token::format_card_id(
        &crate::token::mint().map_err(|e| MintError::Mint(e.to_string()))?,
        None,
        false,
    );
    let mut text = format!("## {front} <!-- id: {token} -->\n");
    for line in &back {
        text.push_str(line);
        text.push('\n');
    }
    let cards = crate::parser::parse_str(deck_id, &text)
        .map_err(|e| MintError::Malformed(e.to_string()))?;
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
            .virtual_ids_with_content(deck_id, fingerprint)
            .is_empty()
    {
        return Err(MintError::Duplicate);
    }
    store.insert_virtual(VirtualCard {
        id: id.clone(),
        kind: VirtualKind::Tutor,
        deck: deck_id.to_string(),
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
pub fn note_badges(store: &mut Store, deck_id: &str, cards: &[Card], now_ms: u64) {
    for depth in [Depth::Recognize, Depth::Recall, Depth::Reconstruct] {
        if store.badge_earned(deck_id, depth).is_some() || !badge_solid(cards, store, depth) {
            continue;
        }
        let entry = store.decks.entry(deck_id.to_string()).or_default();
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
    let parent = vc.deck.clone();

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
                if crate::parser::closes_fence(raw, ch) {
                    fence = None;
                }
            }
            None => {
                if let Some(ch) = crate::parser::fence_opener(raw) {
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
    deck_id: &str,
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
        // The id rides the `## ` line so the stored text re-parses to the same id forever.
        let token = crate::token::format_card_id(
            &crate::token::mint().map_err(|e| anyhow::anyhow!("cannot mint a token: {e}"))?,
            None,
            false,
        );
        let block = stamp_block(block, &token);
        // A malformed block is a hard error, not a silently-dropped card.
        let cards = crate::parser::parse_str(deck_id, &block)?;
        let Some(first) = cards.first() else {
            continue;
        };
        // Fingerprint includes the literal `\blank{}` markers, so a plain card
        // repeating a hole's hidden text can't collide with it.
        let fingerprint = first.content_fingerprint;
        if deck_fingerprints.contains(&fingerprint) {
            continue;
        }
        let existing = store.virtual_ids_with_content(deck_id, fingerprint);
        if existing.is_empty() {
            for card in &cards {
                let Some(id) = card.id() else {
                    continue;
                };
                store.insert_virtual(VirtualCard {
                    id: id.clone(),
                    kind: VirtualKind::Remediation,
                    deck: deck_id.to_string(),
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
    let cards = crate::parser::parse_str(&vc.deck, &vc.text).ok()?;
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

pub fn default_store_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "alix").map(|dirs| dirs.data_dir().to_path_buf())
}

// Syncthing's own naming convention for conflict copies: `<stem>.sync-conflict-*.<ext>`.
pub fn sync_conflicts(store_path: &Path) -> Vec<PathBuf> {
    let direct = store_path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "progress")
        && store_path
            .extension()
            .is_some_and(|extension| extension == "json");
    let mut out = if direct {
        conflict_copies(store_path)
    } else {
        let progress = if store_path
            .file_name()
            .is_some_and(|name| name == "progress")
        {
            store_path.to_path_buf()
        } else {
            store_path.join("progress")
        };
        conflict_documents(&progress)
    };
    out.sort();
    out.dedup();
    out
}

fn conflict_documents(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(crate::workspace::is_conflict_name)
        })
        .collect();
    out.sort();
    out
}

fn conflict_copies(store_path: &Path) -> Vec<PathBuf> {
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
    crate::fsio::create_dir_all(dir).ok()?;
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
        let path = dir.path().join("deck1.json");
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
    fn deleting_a_hole_leaves_exactly_that_record_orphaned() {
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
    fn a_fresh_hole_wins_the_live_key_and_the_stored_hole_is_orphaned() {
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
        let token = "card-tok";
        let a = hf(1, 10);
        let b = hf(2, 20);
        store.ensure_records_raw(token, &[a, b]);
        store.get_or_insert("card-tok-0", 0).total_reviews = 1;
        store.get_or_insert("card-tok-1", 0).total_reviews = 2;

        let z = hf(8, 80);
        let outcome = store.realign_card_holes(token, &[z, b]).unwrap();
        assert_eq!(vec![(1, 1)], outcome.remap);
        assert_eq!(vec![0], outcome.orphaned);
        assert_eq!(vec![0], outcome.fresh);

        assert_eq!(2, store.get("card-tok-1").unwrap().total_reviews);
        assert!(
            store.get("card-tok-0").is_none(),
            "the orphaned hole is deleted"
        );
        assert_eq!(vec![z, b], store.records(token).unwrap().holes);
    }

    #[test]
    fn a_stray_high_index_hole_entry_is_pulled_by_the_cascade_not_left_to_squat() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let token = "card-tok";
        let a = hf(1, 10);
        let b = hf(2, 20);
        store.ensure_records_raw(token, &[a, b]);
        store.get_or_insert("card-tok-0", 0).total_reviews = 1;
        store.get_or_insert("card-tok-1", 0).total_reviews = 2;
        store.get_or_insert("card-tok-5", 0).total_reviews = 9;

        let outcome = store.realign_card_holes(token, &[b, a]).unwrap();
        assert_eq!(vec![(0, 1), (1, 0)], outcome.remap);

        assert_eq!(
            2,
            store.get("card-tok-0").unwrap().total_reviews,
            "b -> hole 0"
        );
        assert_eq!(
            1,
            store.get("card-tok-1").unwrap().total_reviews,
            "a -> hole 1"
        );
        assert!(
            store.get("card-tok-5").is_none(),
            "the stray is deleted, not left under a live key"
        );
    }

    #[test]
    fn a_stale_fingerprint_version_is_ignored_and_rewritten_never_mismatched() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let token = "card-tok";
        let a = hf(1, 10);
        let b = hf(2, 20);
        store.records.insert(
            token.to_string(),
            CardRecords {
                version: FP_VERSION.wrapping_add(1),
                holes: vec![a],
            },
        );
        store.get_or_insert("card-tok-0", 0).total_reviews = 7;

        let outcome = store.realign_card_holes(token, &[a, b]);
        assert!(outcome.is_none());
        assert_eq!(7, store.get("card-tok-0").unwrap().total_reviews);
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
            id: "card-vq".to_string(),
            kind: VirtualKind::Remediation,
            deck: "d1".to_string(),
            text: "## v <!-- id: card-vq -->\nb\n".to_string(),
            created_ms: 0,
        });
        store.get_or_insert("card-vq", 0);
        store.set_last_depth("d1", Depth::Recall);
        store.set_last_depth("d2", Depth::Recall);

        let known_cards: HashSet<String> = ["live".to_string()].into_iter().collect();
        let known_deck_ids: HashSet<String> = ["d1".to_string()].into_iter().collect();
        let orphans = store.orphans(&known_cards, &known_deck_ids);
        assert_eq!(vec!["gone".to_string()], orphans.cards);
        assert_eq!(vec!["d2".to_string()], orphans.decks);
        assert_eq!(2, orphans.len());

        assert_eq!(2, store.prune_orphans(&orphans));
        assert!(store.get("live").is_some());
        assert!(store.get("card-vq").is_some());
        assert_eq!(Some(Depth::Recall), store.last_depth("d1"));
        assert!(store.get("gone").is_none());
        assert_eq!(None, store.last_depth("d2"));
    }

    #[test]
    fn reset_orphans_clears_records_of_pruned_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let a = hf(1, 10);

        store.get_or_insert("card-gonetoken", 0).total_reviews = 3;
        store.ensure_records_raw("card-gonetoken", &[a]);
        store.get_or_insert("card-livetoken", 0).total_reviews = 7;
        store.ensure_records_raw("card-livetoken", &[a]);

        let known_cards: HashSet<String> = ["card-livetoken".to_string()].into_iter().collect();
        let orphans = store.orphans(&known_cards, &HashSet::new());
        assert_eq!(vec!["card-gonetoken".to_string()], orphans.cards);

        store.prune_orphans(&orphans);

        assert!(store.get("card-gonetoken").is_none());
        assert!(store.records("card-gonetoken").is_none());
        assert!(store.get("card-livetoken").is_some());
        assert!(store.records("card-livetoken").is_some());
    }

    #[test]
    fn wipe_deck_clears_every_family_for_its_tokens_and_spares_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let a = hf(1, 10);

        store.get_or_insert("card-doom", 0);
        store.get_or_insert("card-doom-0", 0);
        store.ensure_records_raw("card-doom", &[a]);
        store.set_deck_mastered("doomed", 1);
        store.insert_virtual(VirtualCard {
            id: "card-vdoom".to_string(),
            kind: VirtualKind::Remediation,
            deck: "doomed".to_string(),
            text: "## v <!-- id: card-vdoom -->\nx\n".to_string(),
            created_ms: 0,
        });
        store.get_or_insert("card-vdoom", 0);
        store.get_or_insert("keep", 0);
        store.ensure_records_raw("keep", &[a]);
        store.set_deck_mastered("keep", 1);

        let tokens: HashSet<String> = ["card-doom".to_string()].into_iter().collect();
        let wiped = store.wipe_deck(&tokens, "doomed");

        assert_eq!(2, wiped, "the base and the hole schedule both count");
        assert!(store.get("card-doom").is_none());
        assert!(store.get("card-doom-0").is_none());
        assert!(store.records("card-doom").is_none());
        assert!(!store.deck_mastered("doomed"));
        assert!(store.get_virtual("card-vdoom").is_none());
        assert!(store.get("card-vdoom").is_none());
        assert!(store.get("keep").is_some());
        assert!(store.records("keep").is_some());
        assert!(store.deck_mastered("keep"));
    }

    #[test]
    fn save_stamps_the_writer_and_a_reopen_sees_it_as_foreign_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
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
        let path = dir.path().join("deck1.json");
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
        let path = dir.path().join("deck1.json");
        Store::open(&path).unwrap().save().unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.foreign_writer("phone-1", 0).is_none());
    }

    #[test]
    fn the_warn_window_separates_roaming_from_concurrent_writes() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
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
        std::fs::create_dir(dir.path().join("progress")).unwrap();
        let store_path = dir.path().join("progress/deck1.json");
        std::fs::write(&store_path, "{}").unwrap();
        let conflict = dir
            .path()
            .join("progress/deck1.sync-conflict-20260714-101112-ABCDEF7.json");
        std::fs::write(&conflict, "{}").unwrap();
        for near_miss in [
            "progress/recent.sync-conflict-20260714-101112-AAAAAAA.json",
            "progress/deck1.sync-conflict-20260714.txt",
            "progress/deck1.json.tmp",
        ] {
            std::fs::write(dir.path().join(near_miss), "{}").unwrap();
        }
        assert_eq!(sync_conflicts(&store_path), vec![conflict]);
        assert_eq!(
            sync_conflicts(&dir.path().join("missing/progress/deck1.json")),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn sync_conflicts_finds_per_deck_progress_copies() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("progress")).unwrap();
        let progress = dir
            .path()
            .join("progress/deck1.sync-conflict-20260714-phone.json");
        std::fs::write(&progress, "{}").unwrap();

        assert_eq!(sync_conflicts(dir.path()), vec![progress]);
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
        let path = dir.path().join("deck1.json");
        std::fs::write(&path, "this is not json").unwrap();
        assert!(Store::open(&path).is_err());
    }

    #[test]
    fn open_keeps_a_card_key_of_any_charset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        let mut store = Store::open(&path).unwrap();
        store.get_or_insert("not-a-token", 0);
        store.save().unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.get("not-a-token").is_some());
    }

    #[test]
    fn last_review_ms_is_the_latest_across_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
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
        let path = dir.path().join("deck1.json");
        let store = Store::open(&path).unwrap();
        assert_eq!(path.as_path(), store.path());
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");

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
    fn deck_document_roundtrip_records_owner_and_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress/deck1.json");
        let mut store = Store::open_deck(&path, "deck1", "old.md").unwrap();
        store.get_or_insert("card1", 1);
        store.set_deck_mastered("deck1", 2);
        store.save().unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\": 1"));
        assert!(text.contains("\"deck_id\": \"deck1\""));
        assert!(text.contains("\"revision\": 1"));

        let reopened = Store::open_deck(&path, "deck1", "old.md").unwrap();
        assert!(reopened.get("card1").is_some());
        assert!(reopened.deck_mastered("deck1"));
    }

    #[test]
    fn opening_a_deck_document_under_a_new_filename_keeps_its_deck_level_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress/deck1.json");
        let mut store = Store::open_deck(&path, "deck1", "old.md").unwrap();
        store.set_deck_mastered("deck1", 2);
        store.insert_virtual(VirtualCard {
            id: "card-virtual1".to_string(),
            kind: VirtualKind::Tutor,
            deck: "deck1".to_string(),
            text: "## q <!-- id: card-virtual1 -->\na\n".to_string(),
            created_ms: 1,
        });
        store.save().unwrap();

        // Deck-level state and virtual-card association follow the id, so a
        // reopen under a new filename (same document, same deck_id) still
        // finds them: the subject argument no longer rebinds anything.
        let renamed = Store::open_deck(&path, "deck1", "new.md").unwrap();
        assert!(renamed.deck_mastered("deck1"));
        assert_eq!(
            vec!["card-virtual1"],
            renamed
                .virtual_cards_for("deck1")
                .into_iter()
                .map(|card| card.id.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_deck_document_refuses_the_wrong_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress/deck1.json");
        Store::open_deck(&path, "deck1", "d.md")
            .unwrap()
            .save()
            .unwrap();

        let error = match Store::open_deck(&path, "deck2", "d.md") {
            Ok(_) => panic!("wrong owner was accepted"),
            Err(error) => error,
        };
        assert!(matches!(error, StoreError::DeckOwner { .. }));
    }

    #[test]
    fn a_stale_deck_document_save_does_not_replace_the_newer_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress/deck1.json");
        Store::open_deck(&path, "deck1", "d.md")
            .unwrap()
            .save()
            .unwrap();
        let mut first = Store::open_deck(&path, "deck1", "d.md").unwrap();
        let mut stale = Store::open_deck(&path, "deck1", "d.md").unwrap();
        first.get_or_insert("newer", 1);
        first.save().unwrap();
        stale.get_or_insert("stale", 1);

        let error = stale.save().unwrap_err();
        assert!(matches!(
            error,
            StoreError::StaleRevision {
                loaded: 1,
                disk: 2,
                ..
            }
        ));
        let reopened = Store::open_deck(&path, "deck1", "d.md").unwrap();
        assert!(reopened.get("newer").is_some());
        assert!(reopened.get("stale").is_none());
    }

    #[test]
    fn an_unrecognized_field_is_rejected_loudly_not_silently_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress/deck1.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // A renamed-away or removed field must fail the load, not be ignored
        // and then dropped when the next save rewrites the document.
        std::fs::write(
            &path,
            r#"{"version":1,"deck_id":"deck1","subject":"d.md","revision":1,"cards":{},"retired_field":true}"#,
        )
        .unwrap();

        let error = match Store::open_deck(&path, "deck1", "d.md") {
            Ok(_) => panic!("an unknown field was silently accepted"),
            Err(error) => error,
        };
        assert!(matches!(error, StoreError::Format { .. }));
    }

    #[test]
    fn a_malformed_virtual_card_fails_the_load_instead_of_vanishing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress/deck1.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Missing the required parent/text/created_ms: it used to be decoded
        // with `.ok()` and silently dropped; now the whole load fails loudly.
        std::fs::write(
            &path,
            r#"{"version":1,"deck_id":"deck1","subject":"d.md","revision":1,"cards":{},"virtual_cards":{"v1":{"id":"v1","kind":"tutor"}}}"#,
        )
        .unwrap();

        let error = match Store::open_deck(&path, "deck1", "d.md") {
            Ok(_) => panic!("a malformed virtual card was silently dropped"),
            Err(error) => error,
        };
        assert!(matches!(error, StoreError::Format { .. }));
    }

    #[test]
    fn a_truncated_progress_document_is_rejected_and_a_fresh_save_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress/deck1.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // A save cut short by a crash before atomic replacement can never
        // produce this, but a corrupted sync copy can; opening must not panic.
        std::fs::write(&path, r#"{"version":1,"deck_id":"deck1","revi"#).unwrap();

        let error = match Store::open_deck(&path, "deck1", "d.md") {
            Ok(_) => panic!("a truncated document was accepted"),
            Err(error) => error,
        };
        assert!(matches!(error, StoreError::Format { .. }));

        // The document is not the only copy of anything yet; a fresh save lands.
        std::fs::remove_file(&path).unwrap();
        let mut fresh = Store::open_deck(&path, "deck1", "d.md").unwrap();
        fresh.get_or_insert("card1", 1);
        fresh.save().unwrap();
        assert!(
            Store::open_deck(&path, "deck1", "d.md")
                .unwrap()
                .get("card1")
                .is_some()
        );
    }

    #[test]
    fn propagated_flag_survives_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");

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
        let path = dir.path().join("deck1.json");

        let mut store = Store::open(&path).unwrap();
        assert!(!store.deck_mastered("deck1"));
        assert_eq!(None, store.deck_mastered_at("deck1"));
        store.set_deck_mastered("deck1", 1234);
        assert!(store.deck_mastered("deck1"));
        assert_eq!(Some(1234), store.deck_mastered_at("deck1"));
        store.save().unwrap();

        let mut reloaded = Store::open(&path).unwrap();
        assert!(reloaded.deck_mastered("deck1"));
        assert_eq!(Some(1234), reloaded.deck_mastered_at("deck1"));
        assert!(reloaded.clear_deck_mastered("deck1"));
        assert!(!reloaded.deck_mastered("deck1"));
        assert!(!reloaded.clear_deck_mastered("deck1"));
    }

    #[test]
    fn exam_failed_records_and_a_pass_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");

        let mut store = Store::open(&path).unwrap();
        assert_eq!(None, store.exam_failed_at("deck1"));
        store.set_exam_failed("deck1", 5000);
        assert_eq!(Some(5000), store.exam_failed_at("deck1"));
        assert!(!store.deck_mastered("deck1"));
        store.save().unwrap();

        let mut reloaded = Store::open(&path).unwrap();
        assert_eq!(Some(5000), reloaded.exam_failed_at("deck1"));
        reloaded.set_deck_mastered("deck1", 9000);
        assert!(reloaded.deck_mastered("deck1"));
        assert_eq!(None, reloaded.exam_failed_at("deck1"));
    }

    #[test]
    fn per_deck_clear_drops_the_cooldown_too() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.set_exam_failed("t", 1);
        assert!(store.clear_deck_mastered("t"));
        assert_eq!(None, store.exam_failed_at("t"));
    }

    #[test]
    fn cooldown_remaining_is_none_for_a_deck_that_never_failed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("p.json")).unwrap();
        assert_eq!(None, cooldown_remaining_ms(&store, "t", 3600, 0));
    }

    #[test]
    fn cooldown_remaining_is_none_when_the_cooldown_is_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.set_exam_failed("t", 1_000);
        assert_eq!(None, cooldown_remaining_ms(&store, "t", 0, 1_030_000));
    }

    #[test]
    fn cooldown_remaining_reports_the_exact_ms_left_inside_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.set_exam_failed("t", 1_000);
        let now = 1_000 + 30_000;
        assert_eq!(
            Some(3_600_000 - 30_000),
            cooldown_remaining_ms(&store, "t", 3600, now)
        );
    }

    #[test]
    fn cooldown_remaining_is_none_once_the_window_has_elapsed() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.set_exam_failed("t", 1_000);
        assert_eq!(
            None,
            cooldown_remaining_ms(&store, "t", 3600, 1_000 + 3_600_001)
        );
    }

    #[test]
    fn clear_also_drops_deck_mastered() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.set_deck_mastered("a", 1);
        store.clear();
        assert!(!store.deck_mastered("a"));
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

    const BORROW_TEXT: &str = "## What does the borrow checker enforce? <!-- id: card-vb1 -->\nExactly one mutable borrow, or many shared ones\n";

    fn virtual_card(deck_id: &str, text: &str) -> VirtualCard {
        let id = crate::parser::parse_str(deck_id, text).unwrap()[0]
            .id()
            .unwrap();
        VirtualCard {
            id,
            kind: VirtualKind::Remediation,
            deck: deck_id.to_string(),
            text: text.to_string(),
            created_ms: 1000,
        }
    }

    #[test]
    fn insert_virtual_then_get_virtual_returns_it_with_fields_intact() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let vc = virtual_card("rust", BORROW_TEXT);
        let id = vc.id.clone();

        store.insert_virtual(vc);

        let got = store.get_virtual(&id).unwrap();
        assert_eq!("rust", got.deck);
        assert_eq!(VirtualKind::Remediation, got.kind);
        assert_eq!(BORROW_TEXT, got.text);
        assert!(store.is_virtual(&id));
    }

    #[test]
    fn virtual_card_survives_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        let mut store = Store::open(&path).unwrap();
        // A per-deck document normalizes every virtual card's `deck` to its
        // own id on load, so the fixture must already agree with `deck1`.
        let vc = virtual_card("deck1", BORROW_TEXT);
        let id = vc.id.clone();
        store.insert_virtual(vc.clone());
        store.save().unwrap();

        let reloaded = Store::open(&path).unwrap();
        let got = reloaded.get_virtual(&id).unwrap();
        assert_eq!(&vc, got);
    }

    #[test]
    fn virtual_cards_for_matches_on_owning_deck_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.insert_virtual(virtual_card(
            "rust",
            "## f <!-- id: card-v1 -->\nback one\n",
        ));
        store.insert_virtual(virtual_card(
            "rust",
            "## f <!-- id: card-v2 -->\nback two\n",
        ));
        store.insert_virtual(virtual_card(
            "other",
            "## f <!-- id: card-v3 -->\nback one\n",
        ));

        let rust_cards = store.virtual_cards_for("rust");
        assert_eq!(2, rust_cards.len());
        assert!(rust_cards.iter().all(|v| v.deck == "rust"));

        assert_eq!(1, store.virtual_cards_for("other").len());
        assert!(store.virtual_cards_for("nonexistent").is_empty());
    }

    #[test]
    fn loads_store_file_without_virtual_cards_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        std::fs::write(
            &path,
            r#"{"version":1,"deck_id":"deck1","subject":"deck1.md","revision":1,"cards":{}}"#,
        )
        .unwrap();
        let store = Store::open(&path).unwrap();
        assert!(store.is_empty());
        assert!(store.get_virtual("123").is_none());
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
            "## existing <!-- id: card-ex1 -->\nanswer\n",
        );
        let store_path = dir.path().join("deck1.json");
        let mut store = Store::open(&store_path).unwrap();
        let vc = virtual_card("rust.md", BORROW_TEXT);
        let id = vc.id.clone();
        store.insert_virtual(vc);

        promote_virtual(&mut store, &id, &deck_path).unwrap();

        assert!(store.get_virtual(&id).is_none());

        let text = std::fs::read_to_string(&deck_path).unwrap();
        let cards = crate::parser::parse_str("rust.md", &text).unwrap();
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
            "## one <!-- id: card-q1 -->\n1\n\n## two <!-- id: card-q2 -->\n2\n",
        );
        let before =
            crate::parser::parse_str("rust.md", &std::fs::read_to_string(&deck_path).unwrap())
                .unwrap();
        let ids_before: Vec<String> = before.iter().map(|c| c.id().unwrap()).collect();

        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
        let vc = virtual_card("rust.md", BORROW_TEXT);
        let id = vc.id.clone();
        store.insert_virtual(vc);

        promote_virtual(&mut store, &id, &deck_path).unwrap();

        let after =
            crate::parser::parse_str("rust.md", &std::fs::read_to_string(&deck_path).unwrap())
                .unwrap();
        let ids_after: Vec<String> = after.iter().take(2).map(|c| c.id().unwrap()).collect();
        assert_eq!(ids_before, ids_after);
        assert_eq!(3, after.len());
    }

    #[test]
    fn promote_unknown_id_errors_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = write_deck(dir.path(), "d.md", "## one\n1\n");
        let deck_before = std::fs::read_to_string(&deck_path).unwrap();
        let store_path = dir.path().join("deck1.json");
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
            "## existing <!-- id: card-ex1 -->\nanswer\n",
        );
        let store_path = dir.path().join("deck1.json");
        let mut store = Store::open(&store_path).unwrap();

        let text = "## Complete the quote <!-- id: card-vcz1 -->\nTo \\blank{be} or not to \\blank{be}\n> Hamlet\n";
        let cards = crate::parser::parse_str("rust.md", text).unwrap();
        assert_eq!(2, cards.len());
        let id0 = cards[0].id().unwrap();
        let id1 = cards[1].id().unwrap();
        assert_ne!(id0, id1, "the two holes must have distinct ids");

        for id in [id0.clone(), id1.clone()] {
            store.insert_virtual(VirtualCard {
                id: id.clone(),
                kind: VirtualKind::Remediation,
                deck: "rust.md".to_string(),
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
        let deck_cards = crate::parser::parse_str("rust.md", &deck_text).unwrap();
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
            "## existing <!-- id: card-ex1 -->\nanswer\n",
        );
        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
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
        let cards = crate::parser::parse_str("rust.md", &text).unwrap();
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
        let path = dir.path().join("deck1.json");
        let mut store = Store::open(&path).unwrap();
        let text = "## capital of france <!-- id: card-cap1 -->\nParis\n".to_string();
        let id = crate::parser::parse_str("geo.md", &text).unwrap()[0]
            .id()
            .unwrap();
        store.insert_virtual(VirtualCard {
            id: id.clone(),
            kind: VirtualKind::Tutor,
            deck: "geo.md".to_string(),
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
        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
        let st = store.get_or_insert("7", 1_000);
        *st.schedule_slot(Depth::Reconstruct).unwrap() = Some(FsrsState {
            stability: 4.5,
            ..Default::default()
        });
        st.recognized_ms = Some(2_000);
        st.record_review(2_000, Grade::Pass, Depth::Reconstruct, false);
        store.save().unwrap();
        let reloaded = Store::open(dir.path().join("deck1.json")).unwrap();
        let st = reloaded.get("7").unwrap();
        assert_eq!(
            Some(4.5),
            st.schedule(Depth::Reconstruct).map(|f| f.stability)
        );
        assert_eq!(Some(2_000), st.recognized_ms);
        assert_eq!(Depth::Reconstruct, st.history[0].depth);
    }

    #[test]
    fn history_grades_survive_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
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
        let path = dir.path().join("deck1.json");
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
        crate::parser::parse_str(
            "t.md",
            "## a <!-- id: card-q1 -->\n1\n\n## b <!-- id: card-q2 -->\n2\n",
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
        note_badges(&mut store, "t", &cards, 1_000);
        assert_eq!(Some(1_000), store.badge_earned("t", Depth::Recall));

        store.get_or_insert(&cards[0].id().unwrap(), 0).recall = Some(FsrsState {
            stability: 3.0,
            ..Default::default()
        });

        assert!(!badge_solid(&cards, &store, Depth::Recall));
        assert_eq!(Some(1_000), store.badge_earned("t", Depth::Recall));
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
        assert_eq!(None, store.last_depth("p"));

        store.set_last_depth("p", Depth::Reconstruct);
        assert_eq!(Some(Depth::Reconstruct), store.last_depth("p"));
        store.save().unwrap();

        let reloaded = Store::open(&path).unwrap();
        assert_eq!(Some(Depth::Reconstruct), reloaded.last_depth("p"));
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
            "## Fill\nthe \\blank{a} and \\blank{b}\n",
            300,
            None,
        )
        .unwrap();
        let cloze_id = store
            .virtual_cards_for("d.md")
            .into_iter()
            .find(|v| v.text.contains("\\blank"))
            .unwrap()
            .id
            .clone();
        let (base, _, _) = crate::token::parse_prefixed_card_id(&cloze_id).unwrap();
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
            "---\nsource: x\n---\n\n## Complete the quote\nTo \\blank{be} or not to \\blank{be}\n";
        let blocks = split_card_blocks(text);
        assert_eq!(1, blocks.len());
        assert!(blocks[0].starts_with("## Complete the quote"));
        assert!(blocks[0].contains("\\blank{be}"));
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
        let text = "## Complete the quote\nTo \\blank{be} or not to \\blank{be}\n";

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
        let text = "## Complete the quote\nTo \\blank{be} or not to \\blank{bee}\n";
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
            crate::parser::parse_str("d.md", "## Complete the quote <!-- id: card-p1 -->\nbe\n")
                .unwrap();
        let deck_fingerprints: std::collections::HashSet<u64> =
            plain.iter().map(|c| c.content_fingerprint).collect();

        let cloze = "## Complete the quote\nTo \\blank{be} or not to \\blank{bee}\n";
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
            "## Complete the quote\nTo \\blank{be} or not to \\blank{bee}\n",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let deck_path = dir.path().join("d.md");
            std::fs::write(&deck_path, "## existing <!-- id: card-ex1 -->\nanswer\n").unwrap();
            let mut store = Store::open(dir.path().join("p.json")).unwrap();

            let created = store_remediation(&mut store, "d.md", text, 1_000, None).unwrap();
            let virtuals = store.virtual_cards_for("d.md");
            assert_eq!(created, virtuals.len());

            for vc in &virtuals {
                let synth = crate::parser::parse_str(&vc.deck, &vc.text)
                    .unwrap()
                    .into_iter()
                    .find(|c| c.id().as_deref() == Some(vc.id.as_str()))
                    .expect("synth reproduces the same id");
                assert_eq!(vc.id, synth.id().unwrap());
            }

            let vid = virtuals[0].id.clone();
            promote_virtual(&mut store, &vid, &deck_path).unwrap();
            let deck =
                crate::parser::parse_str("d.md", &std::fs::read_to_string(&deck_path).unwrap())
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
                crate::parser::parse_str(&vc.deck, &vc.text)
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

    #[test]
    fn the_foreign_write_warn_window_is_one_hour() {
        assert_eq!(3_600_000, FOREIGN_WRITE_WARN_WINDOW_MS);
    }

    #[test]
    fn an_overfull_history_from_disk_trims_to_the_cap_on_the_next_review() {
        let mut state = CardState::new(0);
        for i in 0..(HISTORY_CAP + 5) {
            state.history.push(Review {
                ts_ms: i as u64,
                grade: Grade::Pass,
                depth: Depth::Recall,
                propagated: false,
            });
        }
        state.record_review(999_999, Grade::Pass, Depth::Recall, false);
        assert_eq!(HISTORY_CAP, state.history.len());
    }

    #[test]
    fn same_word_holes_on_different_lines_realign_by_line_not_first_come() {
        let x = hf(1, 10);
        let y = hf(1, 20);
        let outcome = realign_holes(&[x, y], &[y, x]);
        assert_eq!(vec![(0, 1), (1, 0)], outcome.remap);
        assert!(outcome.fresh.is_empty(), "{:?}", outcome.fresh);
        assert!(outcome.orphaned.is_empty(), "{:?}", outcome.orphaned);
    }

    #[test]
    fn an_unowned_key_fails_the_save_guard() {
        let mut values: HashMap<String, u32> = HashMap::new();
        values.insert("card-stray".into(), 1);
        let mut owners: HashMap<String, String> = HashMap::new();
        assert!(reject_unowned(&values, &owners, "card").is_err());
        owners.insert("card-stray".into(), "deck-a".into());
        assert!(reject_unowned(&values, &owners, "card").is_ok());
    }

    #[test]
    fn aggregate_open_skips_non_json_and_conflict_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("deck-a.json"),
            r#"{"version":1,"deck_id":"deck-a","subject":"a.md","revision":1,"cards":{"card-a1":{}}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a store document").unwrap();
        std::fs::write(
            dir.path().join("deck-x.sync-conflict-20260802.json"),
            "not json either",
        )
        .unwrap();

        let store = Store::open(dir.path()).unwrap();
        assert_eq!(1, store.len());
    }

    #[test]
    fn the_latest_writer_wins_and_a_timestamp_tie_keeps_the_first_document() {
        let dir = tempfile::tempdir().unwrap();
        let doc = |deck: &str, device: &str, at_ms: u64| {
            format!(
                r#"{{"version":1,"deck_id":"{deck}","subject":"s.md","revision":1,"cards":{{}},"writer":{{"device":"{device}","at_ms":{at_ms}}}}}"#
            )
        };
        std::fs::write(dir.path().join("deck-a.json"), doc("deck-a", "alpha", 100)).unwrap();
        std::fs::write(dir.path().join("deck-b.json"), doc("deck-b", "beta", 200)).unwrap();
        std::fs::write(dir.path().join("deck-c.json"), doc("deck-c", "gamma", 200)).unwrap();

        let store = Store::open(dir.path()).unwrap();
        assert_eq!(
            Some(("beta".to_string(), 800)),
            store.foreign_writer("me", 1_000)
        );
    }

    #[test]
    fn rebinding_one_deck_keeps_every_other_decks_ownership() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("deck-a.json"),
            r#"{"version":1,"deck_id":"deck-a","subject":"a.md","revision":1,"cards":{}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("deck-b.json"),
            r#"{"version":1,"deck_id":"deck-b","subject":"b.md","revision":1,"cards":{"card-b1":{}},"records":{"card-b1":{"version":1,"holes":[]}},"deck":{"last_depth":"recall"}}"#,
        )
        .unwrap();
        let replacement_path = dir.path().join("a2.md");
        std::fs::write(
            &replacement_path,
            "---\nformat-version: 1\nid: \"deck-a2\"\n---\n## q\na\n<!-- id: card-a2c1 -->\n",
        )
        .unwrap();
        let replacement = crate::deck::Deck::load(&replacement_path).unwrap();

        let mut store = Store::open(dir.path()).unwrap();
        store.insert_virtual(virtual_card("deck-b", "## v <!-- id: card-bv1 -->\nkept\n"));
        store.rebind_replaced_deck("deck-a", &replacement).unwrap();
        store.save().unwrap();

        assert!(store.get("card-b1").is_some());
        assert_eq!(Some(Depth::Recall), store.last_depth("deck-b"));
        assert_eq!(1, store.virtual_cards_for("deck-b").len());
    }

    #[test]
    fn removing_a_virtual_block_counts_only_that_decks_exact_text() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.insert_virtual(virtual_card("rust", "## f <!-- id: card-w1 -->\nshared\n"));
        store.insert_virtual(virtual_card("rust", "## f <!-- id: card-w2 -->\nshared\n"));
        store.insert_virtual(virtual_card("rust", "## g <!-- id: card-w3 -->\nother\n"));
        store.insert_virtual(virtual_card("other", "## f <!-- id: card-w4 -->\nshared\n"));
        let removed = store.remove_virtual_block("rust", "## f <!-- id: card-w1 -->\nshared\n");
        assert_eq!(1, removed);
        assert_eq!(3, store.virtual_len());
    }

    #[test]
    fn a_stocked_store_is_not_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert("card-one", 0);
        assert!(!store.is_empty());
    }
}
