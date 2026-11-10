use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Result as AnyResult, bail};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    card::Card,
    deck::Deck,
    depth::{Depth, Launch},
    profile::{self, Counter},
    scheduler::Grade,
};

const HISTORY_CAP: usize = 50;

const DECK_DOCUMENT_VERSION: u32 = 1;

#[cfg(test)]
static SYNC_COMMIT_THREADS: Mutex<Vec<(String, std::thread::ThreadId)>> = Mutex::new(Vec::new());

#[cfg(test)]
pub(crate) fn take_sync_commit_threads(deck_id: &str) -> Vec<std::thread::ThreadId> {
    let mut all = SYNC_COMMIT_THREADS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut matched = Vec::new();
    all.retain(|(committed_deck_id, thread)| {
        if committed_deck_id == deck_id {
            matched.push(*thread);
            false
        } else {
            true
        }
    });
    matched
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Sure,
    Unsure,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub ts_ms: u64,
    pub grade: Grade,
    pub depth: Depth,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub propagated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
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
    // None: never acknowledged. An entry can still exist, because grading
    // supplies schedule and history without introducing (ADR 0035).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub introduced_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walked_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walk_grade: Option<Grade>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall: Option<FsrsState>,
    // Depth states are independent on purpose: no cross-crediting between
    // depths (a pass propagates credit downward; the states never merge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconstruct: Option<FsrsState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognize: Option<FsrsState>,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExamRecord {
    pub at_ms: u64,
    pub pass: bool,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExamStamp {
    pub sourced: String,
    pub unsourced_cards: Vec<String>,
}

impl CardState {
    // Materializing an entry INTRODUCES nothing: only the Seen press writes
    // the timestamp (ADR 0035); two constructors that disagreed once made
    // any future get-or-insert path mark a card introduced on the spot.
    pub fn new() -> Self {
        Self {
            introduced_ms: None,
            walked_ms: None,
            walk_grade: None,
            recall: None,
            reconstruct: None,
            recognize: None,
            total_reviews: 0,
            total_passes: 0,
            streak: 0,
            history: Vec::new(),
        }
    }

    // Presentation alone sets none of these: any of them means the learner
    // did something with the card beyond being shown it.
    /// The explicit form the silent-stamping constructor was killed for:
    /// says what it does in its name, so a call site cannot introduce a card
    /// by accident.
    pub fn introduced_at(now_ms: u64) -> Self {
        Self {
            introduced_ms: Some(now_ms),
            ..Self::new()
        }
    }

    pub fn engaged(&self) -> bool {
        self.introduced_ms.is_some()
            || self.walked_ms.is_some()
            || self.scheduled()
            || self.total_reviews > 0
    }

    pub fn scheduled(&self) -> bool {
        self.recognize.is_some() || self.recall.is_some() || self.reconstruct.is_some()
    }

    pub fn schedule(&self, depth: Depth) -> Option<&FsrsState> {
        match depth {
            Depth::Recognize => self.recognize.as_ref(),
            Depth::Recall => self.recall.as_ref(),
            Depth::Reconstruct => self.reconstruct.as_ref(),
        }
    }

    pub fn schedule_slot(&mut self, depth: Depth) -> Option<&mut Option<FsrsState>> {
        match depth {
            Depth::Recognize => Some(&mut self.recognize),
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
            confidence: None,
        });
        let excess = self.history.len().saturating_sub(HISTORY_CAP);
        self.history.drain(..excess);
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeckProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mastered_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exam_stamp: Option<ExamStamp>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub topic_results: BTreeMap<String, ExamRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_exam_result: Option<FullExamResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_launch: Option<Launch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognized_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recalled_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconstructed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub track_confidence: bool,
}

impl DeckProgress {
    pub(crate) fn is_empty(&self) -> bool {
        *self == DeckProgress::default()
    }
}

// Raw verdict counts; points are derived as pass + partial / 2.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FullExamResult {
    pub at_ms: u64,
    pub pass: u32,
    pub partial: u32,
    pub total: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SittingRecord {
    pub topics: Vec<TopicRecord>,
    pub full: bool,
    pub pass: u32,
    pub partial: u32,
    pub total: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TopicRecord {
    pub topic_id: String,
    pub fingerprint: String,
    pub pass: bool,
}

// 21 days: the FSRS-community convention for a "mature" card.
pub const MATURE_STABILITY_DAYS: f64 = 21.0;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Writer {
    pub device: String,
    pub at_ms: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeckStoreFile {
    version: u32,
    deck_id: String,
    // Display only, never a key.
    subject: String,
    revision: u64,
    cards: HashMap<String, CardState>,
    #[serde(default, skip_serializing_if = "DeckProgress::is_empty")]
    deck: DeckProgress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    writer: Option<Writer>,
}

#[cfg(feature = "full")]
#[derive(Clone)]
pub(crate) struct ValidatedDeckDocument(DeckStoreFile);

#[cfg(feature = "full")]
impl ValidatedDeckDocument {
    pub(crate) fn parse(bytes: &[u8], expected_deck_id: &str) -> Result<Self, StoreError> {
        let path = PathBuf::from("sync push body");
        let file: DeckStoreFile =
            serde_json::from_slice(bytes).map_err(|source| StoreError::Format {
                path: path.clone(),
                source,
            })?;
        validate_deck_file(&path, expected_deck_id, &file)?;
        Ok(Self(file))
    }
}

#[cfg(feature = "full")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SyncPushOutcome {
    Accepted {
        revision: u64,
    },
    Conflict {
        desktop_revision: Option<u64>,
        pulled_revision: Option<u64>,
        desktop_writer: Option<Writer>,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StoreDocumentData {
    pub cards: HashMap<String, CardState>,
    pub deck: DeckProgress,
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
    decks: HashMap<String, String>,
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    cards: HashMap<String, CardState>,
    decks: HashMap<String, DeckProgress>,
    // None leaves the existing on-disk writer marker untouched (tests/tools
    // don't masquerade as a device).
    pub device: Option<String>,
    last_writer: Option<Writer>,
    failed_decks: HashMap<String, PathBuf>,
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
    #[error("{path}: progress revision cannot advance past u64::MAX")]
    RevisionExhausted { path: PathBuf },
    #[error("duplicate {kind} key `{key}` across per-deck progress documents")]
    DuplicateKey { kind: &'static str, key: String },
    #[error("cannot save aggregate progress: {kind} key `{key}` has no owning deck")]
    UnownedKey { kind: &'static str, key: String },
    #[error("{subject}: deck is not initialized")]
    MissingDeckId { subject: String },
    #[error("{path}: progress document holds an exam standing without its evidence stamp")]
    StandingWithoutStamp { path: PathBuf },
}

fn validate_deck_file(
    path: &Path,
    expected_deck_id: &str,
    file: &DeckStoreFile,
) -> Result<(), StoreError> {
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
            actual: file.deck_id.clone(),
        });
    }
    standing_has_stamp(path, &file.deck)
}

fn standing_has_stamp(path: &Path, progress: &DeckProgress) -> Result<(), StoreError> {
    let standing = progress.passed_at_ms.is_some() || progress.mastered_at_ms.is_some();
    if standing && progress.exam_stamp.is_none() {
        return Err(StoreError::StandingWithoutStamp {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

#[cfg(feature = "full")]
pub(crate) fn sync_push_document(
    path: &Path,
    expected_deck_id: &str,
    pulled_revision: Option<u64>,
    document: ValidatedDeckDocument,
) -> Result<SyncPushOutcome, StoreError> {
    let existing = match std::fs::read(path) {
        Ok(bytes) => {
            let file: DeckStoreFile =
                serde_json::from_slice(&bytes).map_err(|source| StoreError::Format {
                    path: path.to_path_buf(),
                    source,
                })?;
            validate_deck_file(path, expected_deck_id, &file)?;
            Some((bytes, file.revision, file.writer))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(StoreError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let desktop_revision = existing.as_ref().map(|(_, revision, _)| *revision);
    if desktop_revision != pulled_revision {
        return Ok(SyncPushOutcome::Conflict {
            desktop_revision,
            pulled_revision,
            desktop_writer: existing.and_then(|(_, _, writer)| writer),
        });
    }
    let revision = pulled_revision.unwrap_or(0).checked_add(1).ok_or_else(|| {
        StoreError::RevisionExhausted {
            path: path.to_path_buf(),
        }
    })?;
    #[cfg(test)]
    SYNC_COMMIT_THREADS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push((expected_deck_id.to_string(), std::thread::current().id()));
    if let Some(parent) = path.parent() {
        crate::fsio::create_dir_all(parent).map_err(|source| StoreError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    if let Some((bytes, _, _)) = existing {
        let backup = path.with_extension("json.bak");
        crate::fsio::replace_file(&path.with_extension("json.bak.tmp"), &backup, &bytes).map_err(
            |source| StoreError::Io {
                path: backup,
                source,
            },
        )?;
    }
    let mut file = document.0;
    file.revision = revision;
    write_json_atomic(path, &file).map_err(WriteFailure::into_error)?;
    Ok(SyncPushOutcome::Accepted { revision })
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
    validate_deck_file(path, expected_deck_id, &file)?;
    Ok(file.revision)
}

struct WriteFailure {
    error: StoreError,
    replaced: bool,
}

impl WriteFailure {
    fn into_error(self) -> StoreError {
        self.error
    }
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), WriteFailure> {
    let io_err = |source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    };
    let json = serde_json::to_string_pretty(value).map_err(|source| WriteFailure {
        error: StoreError::Format {
            path: path.to_path_buf(),
            source,
        },
        replaced: false,
    })?;
    let tmp = path.with_extension("json.tmp");
    crate::fsio::replace_file_report(&tmp, path, json.as_bytes()).map_err(|failure| {
        let replaced = failure.replaced();
        WriteFailure {
            error: io_err(failure.into_source()),
            replaced,
        }
    })
}

fn write_deck_data(
    path: &Path,
    deck_id: &str,
    subject: &str,
    revision: u64,
    data: &StoreDocumentData,
) -> Result<(), WriteFailure> {
    if let Some(dir) = path.parent() {
        crate::fsio::create_dir_all(dir).map_err(|source| WriteFailure {
            error: StoreError::Io {
                path: path.to_path_buf(),
                source,
            },
            replaced: false,
        })?;
    }
    let file = DeckStoreFile {
        version: DECK_DOCUMENT_VERSION,
        deck_id: deck_id.to_string(),
        subject: subject.to_string(),
        revision,
        cards: data.cards.clone(),
        deck: data.deck.clone(),
        writer: data.writer.clone(),
    };
    write_json_atomic(path, &file)
}

/// A cheap change stamp over the progress directory: file names, lengths,
/// and modification times of the documents an aggregate open would read,
/// no contents. In-process comparison only; never persisted.
#[cfg(feature = "full")]
pub(crate) fn progress_dir_stamp(progress_dir: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut names: Vec<(String, u64, u128)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(progress_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_document = path.is_file()
                && path.extension().is_some_and(|ext| ext == "json")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_none_or(|name| {
                        !crate::workspace::is_private_name(name)
                            && !crate::workspace::is_conflict_name(name)
                    });
            if !is_document {
                continue;
            }
            let (len, mtime) = entry
                .metadata()
                .map(|meta| {
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos())
                        .unwrap_or_default();
                    (meta.len(), mtime)
                })
                .unwrap_or_default();
            names.push((entry.file_name().to_string_lossy().into_owned(), len, mtime));
        }
    }
    names.sort();
    let mut hasher = std::hash::DefaultHasher::new();
    names.hash(&mut hasher);
    hasher.finish()
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
    validate_deck_file(path, expected_deck_id, &file)?;
    let subject = current_subject.unwrap_or(&file.subject).to_string();
    Ok((
        file.revision,
        subject,
        StoreDocumentData {
            cards: file.cards,
            deck: file.deck,
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
                device: None,
                last_writer: None,
                failed_decks: HashMap::new(),
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
        validate_deck_file(&path, &deck_id, &file)?;

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
            device: None,
            last_writer: file.writer,
            failed_decks: HashMap::new(),
            backing: StoreBacking::Deck {
                deck_id,
                subject,
                revision: AtomicU64::new(file.revision),
            },
        })
    }

    pub fn open_for_decks(path: impl AsRef<Path>, decks: &[Deck]) -> Result<Self, StoreError> {
        Self::open_aggregate_impl(path.as_ref().to_path_buf(), decks, true)
    }

    /// The aggregate open with per-document read granularity: a document
    /// that fails to read is SKIPPED and its deck id collected into
    /// [`Store::progress_error`], instead of failing the whole store, so one
    /// damaged member cannot silence its siblings' progress. Tolerance never
    /// extends to the decks an open is for: an expected deck's own corrupt
    /// document still fails the open, or a session on it would mint fresh
    /// progress over the damaged file. Directory-level failures and
    /// cross-document ownership conflicts still fail the open: tolerance of
    /// a damaged file is never tolerance of a duplicate-key conflict.
    pub fn open_aggregate_tolerant(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_aggregate_impl(path.as_ref().to_path_buf(), &[], true)
    }

    fn open_aggregate_for(path: PathBuf, expected_decks: &[Deck]) -> Result<Self, StoreError> {
        Self::open_aggregate_impl(path, expected_decks, false)
    }

    fn open_aggregate_impl(
        path: PathBuf,
        expected_decks: &[Deck],
        tolerant: bool,
    ) -> Result<Self, StoreError> {
        // The promised distinction: an ABSENT root is a fresh store; an
        // existing non-directory root is damage and must fail loud, or the
        // listing would fabricate fresh-and-due rows over it.
        if path.exists() && !path.is_dir() {
            return Err(StoreError::Io {
                path: path.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "the progress root is not a directory",
                ),
            });
        }
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
                    .is_none_or(|name| {
                        !crate::workspace::is_private_name(name)
                            && !crate::workspace::is_conflict_name(name)
                    })
        });
        document_paths.sort();

        let mut cards = HashMap::new();
        let mut decks = HashMap::new();
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
            }
            owners
                .decks
                .insert(deck_id.to_string(), deck_id.to_string());
        }
        let mut documents = Vec::new();
        let mut loaded_decks = HashSet::new();
        let mut last_writer: Option<Writer> = None;
        let mut failed_decks = HashMap::new();
        for document_path in document_paths {
            let Some(deck_id) =
                crate::state::deck_id_from_document(&document_path).map(str::to_string)
            else {
                continue;
            };
            let current_subject = expected.get(&deck_id).map(String::as_str);
            profile::hit(Counter::StoreDocumentsRead);
            let (revision, subject, data) =
                match read_deck_data(&document_path, &deck_id, current_subject) {
                    Ok(read) => read,
                    Err(error) => {
                        if tolerant && !expected.contains_key(&deck_id) {
                            failed_decks.insert(deck_id, document_path);
                            continue;
                        }
                        return Err(error);
                    }
                };
            merge_owned(&mut cards, &mut owners.cards, &data.cards, &deck_id, "card")?;
            owners.decks.insert(deck_id.clone(), deck_id.clone());
            if !data.deck.is_empty() {
                decks.insert(deck_id.clone(), data.deck.clone());
            }
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
            device: None,
            last_writer,
            failed_decks,
            backing: StoreBacking::Aggregate {
                documents: Mutex::new(documents),
                owners,
            },
        })
    }

    /// True when a tolerant open skipped the deck's progress document as
    /// unreadable: the deck has stored progress that this view cannot see,
    /// so no progress claim about it is honest and no session may write it.
    pub fn progress_error(&self, deck_id: &str) -> bool {
        self.failed_decks.contains_key(deck_id)
    }

    pub fn failed_decks(&self) -> &HashMap<String, PathBuf> {
        &self.failed_decks
    }

    /// Carries the replaced view's damage knowledge into this store for
    /// every document this store's own open did not attempt: a deck store
    /// attempted exactly its own document and an aggregate its own progress
    /// directory, so those verdicts stand (a re-read heals or re-confirms),
    /// while damage seen elsewhere would otherwise vanish from view the
    /// moment a session installs a narrower store.
    pub fn carry_failed_decks(&mut self, from: &Store) {
        for (deck_id, document_path) in &from.failed_decks {
            if self.attempted(document_path) {
                continue;
            }
            self.failed_decks
                .insert(deck_id.clone(), document_path.clone());
        }
    }

    fn attempted(&self, document_path: &Path) -> bool {
        match &self.backing {
            StoreBacking::Deck { .. } => self.path == *document_path,
            StoreBacking::Aggregate { .. } => document_path.parent() == Some(self.path.as_path()),
        }
    }

    /// Overlays the owner's actively held document onto this view: the
    /// owner is authoritative for the deck it holds (its unflushed truth
    /// beats the disk copy), so its cards and deck progress
    /// replace this store's, and its deck sheds any failed entry. View
    /// only: an overlaid store is for row building, never for saving.
    #[cfg(feature = "full")]
    pub(crate) fn overlay_owner(&mut self, owner: &Store) {
        let StoreBacking::Deck { deck_id, .. } = &owner.backing else {
            return;
        };
        for (key, value) in &owner.cards {
            self.cards.insert(key.clone(), value.clone());
        }
        for (key, value) in &owner.decks {
            self.decks.insert(key.clone(), value.clone());
        }
        self.failed_decks.remove(deck_id);
    }

    #[cfg(feature = "full")]
    pub(crate) fn is_aggregate(&self) -> bool {
        matches!(&self.backing, StoreBacking::Aggregate { .. })
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
            deck: self.decks.get(deck_id).cloned().unwrap_or_default(),
            writer: self.writer_for_save(),
        };
        match write_json_atomic(&self.path, &file) {
            Ok(()) => {
                revision.store(next, Ordering::Relaxed);
                Ok(())
            }
            Err(failure) => {
                if failure.replaced {
                    revision.store(next, Ordering::Relaxed);
                }
                Err(failure.into_error())
            }
        }
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
        reject_unowned(&self.decks, &owners.decks, "deck")?;

        let mut documents = documents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut changed = Vec::new();
        for (index, document) in documents.iter().enumerate() {
            let data = StoreDocumentData {
                cards: owned_values(&self.cards, &owners.cards, &document.deck_id),
                deck: self
                    .decks
                    .get(&document.deck_id)
                    .cloned()
                    .unwrap_or_default(),
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
            match write_deck_data(
                &document.path,
                &document.deck_id,
                &document.subject,
                next,
                &data,
            ) {
                Ok(()) => {
                    document.revision = next;
                    document.original = data;
                }
                Err(failure) => {
                    if failure.replaced {
                        document.revision = next;
                        document.original = data;
                    }
                    return Err(failure.into_error());
                }
            }
        }
        Ok(())
    }

    pub fn get(&self, card_id: &str) -> Option<&CardState> {
        self.cards.get(card_id)
    }

    // A default CardState can be materialized without engagement, so
    // queue/on-ramp classification reads this filtered view instead of `get`.
    pub fn progress(&self, card_id: &str) -> Option<&CardState> {
        self.cards.get(card_id).filter(|state| state.engaged())
    }

    // Reflects actual reviews, not merely opening the deck.
    pub fn last_review_ms(&self) -> Option<u64> {
        self.cards
            .values()
            .filter_map(|state| state.history.last().map(|review| review.ts_ms))
            .max()
    }

    pub fn get_or_insert(&mut self, card_id: &str) -> &mut CardState {
        self.cards.entry(card_id.to_string()).or_default()
    }

    pub fn remove(&mut self, card_id: &str) -> bool {
        self.cards.remove(card_id).is_some()
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
                owners.decks.retain(|_, owner| owner != old_deck_id);
                owners
                    .decks
                    .insert(new_deck_id.to_string(), new_deck_id.to_string());
                for card in &deck.cards {
                    if let Some(card_id) = card.id() {
                        owners.cards.insert(card_id, new_deck_id.to_string());
                    }
                }
            }
        }
        Ok(())
    }

    pub fn deck_passed(&self, deck_id: &str) -> bool {
        self.deck_passed_at(deck_id).is_some()
    }

    pub fn deck_passed_at(&self, deck_id: &str) -> Option<u64> {
        self.decks.get(deck_id).and_then(|d| d.passed_at_ms)
    }

    pub fn deck_mastered(&self, deck_id: &str) -> bool {
        self.deck_mastered_at(deck_id).is_some()
    }

    pub fn deck_mastered_at(&self, deck_id: &str) -> Option<u64> {
        self.decks.get(deck_id).and_then(|d| d.mastered_at_ms)
    }

    pub fn full_exam_result(&self, deck_id: &str) -> Option<FullExamResult> {
        self.decks.get(deck_id).and_then(|d| d.full_exam_result)
    }

    pub fn topic_result(&self, deck_id: &str, topic_id: &str) -> Option<&ExamRecord> {
        self.decks
            .get(deck_id)
            .and_then(|d| d.topic_results.get(topic_id))
    }

    pub fn topic_results(&self, deck_id: &str) -> Option<&BTreeMap<String, ExamRecord>> {
        self.decks.get(deck_id).map(|d| &d.topic_results)
    }

    pub fn exam_stamp(&self, deck_id: &str) -> Option<&ExamStamp> {
        self.decks.get(deck_id).and_then(|d| d.exam_stamp.as_ref())
    }

    pub fn set_deck_passed(&mut self, deck_id: &str, now_ms: u64, stamp: &ExamStamp) {
        let entry = self.decks.entry(deck_id.to_string()).or_default();
        entry.passed_at_ms.get_or_insert(now_ms);
        entry.exam_stamp.get_or_insert_with(|| stamp.clone());
    }

    pub fn record_exam_result(
        &mut self,
        deck: &Deck,
        record: &SittingRecord,
        now_ms: u64,
        eligible: bool,
        stamp: &ExamStamp,
    ) -> bool {
        let deck_id = deck.deck_token.clone().unwrap_or_default();
        let topics = crate::exam_status::examined_topics(deck);
        self.record_sitting(&deck_id, &topics, record, now_ms, eligible, stamp)
    }

    /// Returns whether this sitting set the deck's `passed_at_ms`.
    pub fn record_sitting(
        &mut self,
        deck_id: &str,
        deck_topics: &[crate::exam_status::ExamTopic],
        record: &SittingRecord,
        now_ms: u64,
        eligible: bool,
        stamp: &ExamStamp,
    ) -> bool {
        let progress = self.decks.entry(deck_id.to_string()).or_default();
        for topic in &record.topics {
            progress.topic_results.insert(
                topic.topic_id.clone(),
                ExamRecord {
                    at_ms: now_ms,
                    pass: topic.pass,
                    fingerprint: topic.fingerprint.clone(),
                },
            );
        }
        if record.full {
            progress.full_exam_result = Some(FullExamResult {
                at_ms: now_ms,
                pass: record.pass,
                partial: record.partial,
                total: record.total,
            });
        }
        let all_passed = crate::exam_status::status_of(deck_id, deck_topics, self).all_passed();
        let masters = record.full && record.total > 0 && record.pass == record.total;
        let newly_passed = eligible && all_passed && !self.deck_passed(deck_id);
        let progress = self.decks.entry(deck_id.to_string()).or_default();
        if newly_passed {
            progress.passed_at_ms.get_or_insert(now_ms);
        }
        if eligible && masters {
            progress.mastered_at_ms.get_or_insert(now_ms);
        }
        if eligible && (all_passed || masters) {
            progress.exam_stamp = Some(stamp.clone());
        }
        if progress.is_empty() {
            self.decks.remove(deck_id);
        }
        newly_passed
    }

    /// Returns the re-keyed topic ids.
    pub fn keep_results(
        &mut self,
        deck_id: &str,
        before: &BTreeMap<String, String>,
        after: &BTreeMap<String, String>,
        current: &ExamStamp,
    ) -> Vec<String> {
        let Some(progress) = self.decks.get_mut(deck_id) else {
            return Vec::new();
        };
        let mut rekeyed = Vec::new();
        for (topic, old) in before {
            let Some(new) = after.get(topic) else {
                continue;
            };
            if new == old {
                continue;
            }
            if let Some(result) = progress.topic_results.get_mut(topic)
                && result.fingerprint == *old
            {
                result.fingerprint = new.clone();
                rekeyed.push(topic.clone());
            }
        }
        if let Some(stamp) = progress.exam_stamp.as_mut() {
            let pairs: Vec<(&str, &str)> = before
                .iter()
                .map(|(topic, fingerprint)| (topic.as_str(), fingerprint.as_str()))
                .collect();
            let at_before = ExamStamp {
                sourced: crate::exam_status::sourced_hash(&pairs),
                unsourced_cards: current.unsourced_cards.clone(),
            };
            if !crate::exam_status::exam_changed(&at_before, stamp) {
                *stamp = current.clone();
            }
        }
        rekeyed
    }

    #[cfg(test)]
    pub(crate) fn record_full_marks(&mut self, deck: &Deck, now_ms: u64) {
        let topics: Vec<TopicRecord> = crate::exam_status::examined_topics(deck)
            .into_iter()
            .map(|topic| TopicRecord {
                topic_id: topic.topic_id,
                fingerprint: topic.fingerprint,
                pass: true,
            })
            .collect();
        let total = topics.len() as u32;
        let record = SittingRecord {
            topics,
            full: true,
            pass: total,
            partial: 0,
            total,
        };
        let stamp = crate::exam_status::sitting_start(deck, self, now_ms, None).stamp;
        self.record_exam_result(deck, &record, now_ms, true, &stamp);
    }

    pub fn ever_examined(&self, deck: &Deck) -> bool {
        let deck_id = deck.deck_token.as_deref().unwrap_or_default();
        self.decks.get(deck_id).is_some_and(|progress| {
            progress.full_exam_result.is_some() || !progress.topic_results.is_empty()
        })
    }

    pub fn clear_deck_exam(&mut self, deck: &Deck) {
        let deck_id = deck.deck_token.as_deref().unwrap_or_default();
        if let Some(progress) = self.decks.get_mut(deck_id) {
            progress.passed_at_ms = None;
            progress.mastered_at_ms = None;
            progress.full_exam_result = None;
            progress.topic_results.clear();
            progress.exam_stamp = None;
            if progress.is_empty() {
                self.decks.remove(deck_id);
            }
        }
    }

    pub fn remove_deck_progress(&mut self, deck_id: &str) -> bool {
        self.decks.remove(deck_id).is_some()
    }

    pub fn last_launch(&self, deck_id: &str) -> Option<Launch> {
        self.decks.get(deck_id).and_then(|d| d.last_launch)
    }

    pub fn set_last_launch(&mut self, deck_id: &str, launch: Launch) {
        self.decks
            .entry(deck_id.to_string())
            .or_default()
            .last_launch = Some(launch);
    }

    pub fn track_confidence(&self, deck_id: &str) -> bool {
        self.decks.get(deck_id).is_some_and(|d| d.track_confidence)
    }

    pub fn set_track_confidence(&mut self, deck_id: &str, on: bool) {
        let deck = self.decks.entry(deck_id.to_string()).or_default();
        deck.track_confidence = on;
        if deck.is_empty() {
            self.decks.remove(deck_id);
        }
    }

    pub fn badge_earned(&self, deck_id: &str, depth: Depth) -> Option<u64> {
        let deck = self.decks.get(deck_id)?;
        match depth {
            Depth::Recognize => deck.recognized_at_ms,
            Depth::Recall => deck.recalled_at_ms,
            Depth::Reconstruct => deck.reconstructed_at_ms,
        }
    }

    pub fn clear(&mut self) -> usize {
        let n = self.cards.len();
        self.cards.clear();
        self.decks.clear();
        n
    }

    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `known_topic_ids` maps each known deck id to its topic ids; a topic
    /// result under a known deck that the map does not list is an orphan.
    pub fn orphans(
        &self,
        known_card_ids: &HashSet<String>,
        known_deck_ids: &HashSet<String>,
        known_topic_ids: &HashMap<String, HashSet<String>>,
    ) -> Orphans {
        let mut cards: Vec<String> = self
            .cards
            .keys()
            .filter(|k| !known_card_ids.contains(*k))
            .cloned()
            .collect();
        let mut decks: Vec<String> = self
            .decks
            .keys()
            .filter(|k| !known_deck_ids.contains(*k))
            .cloned()
            .collect();
        let mut topics: Vec<(String, String)> = self
            .decks
            .iter()
            .filter(|(deck_id, _)| known_deck_ids.contains(*deck_id))
            .flat_map(|(deck_id, progress)| {
                let known = known_topic_ids.get(deck_id);
                progress
                    .topic_results
                    .keys()
                    .filter(move |topic_id| !known.is_some_and(|known| known.contains(*topic_id)))
                    .map(move |topic_id| (deck_id.clone(), topic_id.clone()))
            })
            .collect();
        cards.sort();
        decks.sort();
        topics.sort();
        Orphans {
            cards,
            decks,
            topics,
        }
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
        for (deck_id, topic_id) in &orphans.topics {
            let Some(progress) = self.decks.get_mut(deck_id) else {
                continue;
            };
            if progress.topic_results.remove(topic_id).is_some() {
                removed += 1;
            }
            if progress.is_empty() {
                self.decks.remove(deck_id);
            }
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
                    .is_some_and(|(token, _, _, _)| tokens.contains(token))
            })
            .cloned()
            .collect();
        let mut wiped = 0;
        for id in doomed {
            if self.cards.remove(&id).is_some() {
                wiped += 1;
            }
        }
        self.decks.remove(deck_id);
        wiped
    }
}

// Never auto-pruned: cleared only by an explicit `alix reset --orphans`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Orphans {
    pub cards: Vec<String>,
    pub decks: Vec<String>,
    pub topics: Vec<(String, String)>,
}

impl Orphans {
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty() && self.decks.is_empty() && self.topics.is_empty()
    }

    pub fn len(&self) -> usize {
        self.cards.len() + self.decks.len() + self.topics.len()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MintError {
    #[error("the drafted card is malformed: {0}")]
    Malformed(String),
    #[error("a card with this content already exists in the deck")]
    Duplicate,
    #[error("cannot mint an identity token: {0}")]
    Mint(String),
    #[error("{0}")]
    DuplicatedTopic(String),
    #[error("{0}")]
    Deck(String),
}

pub struct MintedCard {
    pub id: String,
    /// The exact text appended to the sidecar, stamped with `id`.
    pub block: String,
}

// Dedup is by content, not id: every mint gets a fresh random token, so
// identical content would otherwise mint a duplicate.
pub fn mint_tutor_card(
    store: &mut Store,
    deck_path: &Path,
    deck_id: &str,
    topic_id: &str,
    front: &str,
    back: &[String],
    deck_fingerprints: &std::collections::HashSet<u64>,
) -> Result<String, MintError> {
    refuse_duplicated_topic(deck_path, topic_id)?;
    let minted = mint_personal_card(deck_path, deck_id, topic_id, front, back, deck_fingerprints)?;
    store.get_or_insert(&minted.id);
    Ok(minted.id)
}

/// Mints and appends to the sidecar only; the store is the caller's.
pub fn mint_personal_card(
    deck_path: &Path,
    deck_id: &str,
    topic_id: &str,
    front: &str,
    back: &[String],
    deck_fingerprints: &std::collections::HashSet<u64>,
) -> Result<MintedCard, MintError> {
    let (front, back) = plain_sides(front, back)?;
    let token = crate::token::format_card_id(
        &crate::token::mint().map_err(|e| MintError::Mint(e.to_string()))?,
        None,
        false,
    );
    append_checked_card(
        deck_path,
        deck_id,
        plain_block(&front, &back, topic_id, &token),
        deck_fingerprints,
    )
}

pub fn mint_or_return_kept_card(
    deck_path: &Path,
    deck_id: &str,
    topic_id: &str,
    front: &str,
    back: &[String],
    deck_fingerprints: &std::collections::HashSet<u64>,
) -> Result<MintedCard, MintError> {
    refuse_duplicated_topic(deck_path, topic_id)?;
    match mint_personal_card(deck_path, deck_id, topic_id, front, back, deck_fingerprints) {
        Err(MintError::Duplicate) => {
            kept_card(deck_path, deck_id, topic_id, front, back, deck_fingerprints)
                .ok_or(MintError::Duplicate)
        }
        minted => minted,
    }
}

fn refuse_duplicated_topic(deck_path: &Path, topic_id: &str) -> Result<(), MintError> {
    let deck = Deck::load(deck_path).map_err(|e| MintError::Deck(e.to_string()))?;
    let root = crate::workspace::root_for_deck(deck_path);
    match crate::topic::duplicated_topics(&deck, root.as_deref())
        .into_iter()
        .find(|finding| finding.id == topic_id)
    {
        Some(finding) => Err(MintError::DuplicatedTopic(finding.to_string())),
        None => Ok(()),
    }
}

fn sidecar_cards(deck_id: &str, text: &str) -> Option<Vec<Card>> {
    let parsed = crate::parser::parse_sidecar(deck_id, text).ok()?;
    let cards = parsed.cards.into_iter().map(|parsed| parsed.card);
    Some(cards.chain(parsed.skipped).collect())
}

fn kept_card(
    deck_path: &Path,
    deck_id: &str,
    topic_id: &str,
    front: &str,
    back: &[String],
    deck_fingerprints: &std::collections::HashSet<u64>,
) -> Option<MintedCard> {
    let (front, back) = plain_sides(front, back).ok()?;
    let text = std::fs::read_to_string(crate::personal::sidecar_path(deck_path)).ok()?;
    let cards = sidecar_cards(deck_id, &text)?;
    cards
        .iter()
        .filter(|card| card.front == front && card.back == back)
        .filter(|card| !deck_fingerprints.contains(&card.block_fingerprint))
        .find_map(|card| {
            let id = card.id()?;
            let block = plain_block(&front, &back, topic_id, &id);
            let verbatim = text.starts_with(&block) || text.contains(&format!("\n{block}"));
            verbatim.then_some(MintedCard { block, id })
        })
}

fn plain_sides(front: &str, back: &[String]) -> Result<(String, Vec<String>), MintError> {
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
    Ok((front.to_string(), back))
}

fn plain_block(front: &str, back: &[String], topic_id: &str, id: &str) -> String {
    let mut text = format!("## {front}\n");
    for line in back {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str(&topic_line(topic_id));
    text.push_str(&format!("<!-- id: {id} -->\n"));
    text
}

fn topic_line(topic_id: &str) -> String {
    format!("<!-- topic: {topic_id} -->\n")
}

/// Appends a block another device already minted, keeping its id.
pub fn record_minted_card(
    store: &mut Store,
    deck_path: &Path,
    deck_id: &str,
    block: &str,
    deck_fingerprints: &std::collections::HashSet<u64>,
) -> Result<String, MintError> {
    let minted = append_checked_card(deck_path, deck_id, block.to_string(), deck_fingerprints)?;
    store.get_or_insert(&minted.id);
    Ok(minted.id)
}

fn append_checked_card(
    deck_path: &Path,
    deck_id: &str,
    text: String,
    deck_fingerprints: &std::collections::HashSet<u64>,
) -> Result<MintedCard, MintError> {
    let parsed = crate::parser::parse_sidecar(deck_id, &text)
        .map_err(|e| MintError::Malformed(e.to_string()))?;
    if let Some(problem) = parsed.problems.first() {
        return Err(MintError::Malformed(problem.kind.message().to_string()));
    }
    let [crate::parser::ParsedSidecarCard { card, .. }] = parsed.cards.as_slice() else {
        return Err(MintError::Malformed(
            "expected exactly one card".to_string(),
        ));
    };
    let id = card
        .id()
        .ok_or_else(|| MintError::Malformed("the minted card has no identity token".to_string()))?;
    let fingerprint = card.block_fingerprint;
    if deck_fingerprints.contains(&fingerprint)
        || !personal_ids_with_content(deck_path, deck_id, fingerprint).is_empty()
    {
        return Err(MintError::Duplicate);
    }
    crate::personal::append_cards(deck_path, deck_id, &text)
        .map_err(|e| MintError::Malformed(e.to_string()))?;
    Ok(MintedCard { id, block: text })
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
        state
            .schedule(depth)
            .is_some_and(|fsrs| fsrs.stability >= MATURE_STABILITY_DAYS)
    })
}

// High-water: an already-earned date survives a later drop below the mature line.
// Badges gate nothing here, bookkeeping only, never a lifecycle interaction.
pub fn record_badges(store: &mut Store, deck_id: &str, cards: &[Card], now_ms: u64) {
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

// Preamble before the first `## ` front (frontmatter, prose) is dropped: it belongs to no card.
pub fn split_card_blocks(text: &str) -> Vec<String> {
    let mut open = false;
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    // Tracks fences so a `## ` line inside a code fence doesn't start a bogus block.
    let mut fence: Option<(char, usize)> = None;
    for raw in text.lines() {
        match fence {
            Some((ch, open)) => {
                if crate::parser::closes_fence(raw, ch, open) {
                    fence = None;
                }
            }
            None => {
                if let Some(opened) = crate::parser::fence_opener(raw) {
                    fence = Some(opened);
                } else if let Some((depth, _)) = crate::parser::heading_depth(raw) {
                    if crate::parser::is_card_depth(depth) {
                        blocks.push(vec![raw]);
                        open = true;
                    } else {
                        // A topic owns no card: its heading and prose
                        // belong to no block, so nothing may append to the
                        // card above it.
                        open = false;
                    }
                    continue;
                }
            }
        }
        if open && let Some(current) = blocks.last_mut() {
            current.push(raw);
        }
        // else: preamble before the first front, dropped.
    }
    blocks
        .into_iter()
        .map(|lines| format!("{}\n", lines.join("\n")))
        .collect()
}

/// `created_or_revived` counts store entries touched even when `outcome` is
/// an error, so a failed save still reports the in-memory mutation.
pub struct RemediationStored {
    pub created_or_revived: usize,
    pub outcome: AnyResult<()>,
}

impl RemediationStored {
    pub fn into_result(self) -> AnyResult<usize> {
        self.outcome.map(|()| self.created_or_revived)
    }
}

/// The deck file is never touched. Dedup is by content, not id: each block
/// gets a fresh random token, so a rerun must match by canonical content.
pub fn store_remediation_cards(
    store: &mut Store,
    deck_path: Option<&Path>,
    deck_id: &str,
    topic_id: &str,
    deck_fingerprints: &std::collections::HashSet<u64>,
    cards_text: &str,
    retire_after_days: Option<u32>,
) -> RemediationStored {
    let mut created_or_revived = 0;
    let outcome = store_remediation_blocks(
        store,
        deck_path,
        deck_id,
        topic_id,
        deck_fingerprints,
        cards_text,
        retire_after_days,
        &mut created_or_revived,
    );
    RemediationStored {
        created_or_revived,
        outcome,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the public entry's inputs plus its mutation counter"
)]
fn store_remediation_blocks(
    store: &mut Store,
    deck_path: Option<&Path>,
    deck_id: &str,
    topic_id: &str,
    deck_fingerprints: &std::collections::HashSet<u64>,
    cards_text: &str,
    retire_after_days: Option<u32>,
    created_or_revived: &mut usize,
) -> AnyResult<()> {
    let Some(deck_path) = deck_path else {
        bail!("this sitting has no deck file to write personal cards beside");
    };
    refuse_duplicated_topic(deck_path, topic_id)?;
    let blocks = split_card_blocks(cards_text);
    if blocks.is_empty() {
        bail!("remediation produced no cards to store");
    }

    for block in &blocks {
        // Stamped before storing so the text re-parses to the same id forever.
        let token = crate::token::format_card_id(
            &crate::token::mint().map_err(|e| anyhow::anyhow!("cannot mint a token: {e}"))?,
            None,
            false,
        );
        let block = stamp_block(block, &token);
        // Region stamps are identity: an unstamped generated span parses
        // idless and would never schedule, so the production stamper runs
        // over the block before the parse that schedules it.
        let block = stamp_generated_regions(&block)?;
        // A malformed block is a hard error, not a silently-dropped card.
        let cards = crate::parser::parse_str(deck_id, &block)?;
        let Some(first) = cards.first() else {
            continue;
        };
        // The BLOCK key: one sidecar block is created or revived as a
        // unit, whatever its span count.
        let fingerprint = first.block_fingerprint;
        if deck_fingerprints.contains(&fingerprint) {
            continue;
        }
        let existing = personal_ids_with_content(deck_path, deck_id, fingerprint);
        if existing.is_empty() {
            let Some(block) = bind_block_to_topic(&block, &token, topic_id) else {
                bail!("the stamped remediation block lost its id line `{token}`");
            };
            crate::personal::append_cards(deck_path, deck_id, &block)?;
            for card in &cards {
                let Some(id) = card.id() else {
                    continue;
                };
                store.get_or_insert(&id);
                *created_or_revived += 1;
            }
        } else if existing
            .iter()
            .all(|id| crate::session::is_retired_id(id, store, retire_after_days))
        {
            let bases = personal_bases_with_content(deck_path, deck_id, fingerprint);
            crate::personal::rebind_topic(deck_path, &bases, topic_id)?;
            for id in &existing {
                *store.get_or_insert(id) = CardState::new();
                *created_or_revived += 1;
            }
        }
        // Else at least one matching entry is still active: leave it, no reset.
    }
    if *created_or_revived > 0 {
        store.save()?;
    }
    Ok(())
}

/// Ids of personal cards already in the sidecar whose content matches, so a
/// rerun of the same gap revives the card instead of writing it twice.
fn personal_ids_with_content(deck_path: &Path, deck_id: &str, fingerprint: u64) -> Vec<String> {
    personal_cards_with_content(deck_path, deck_id, fingerprint)
        .iter()
        .filter_map(Card::id)
        .collect()
}

fn personal_bases_with_content(deck_path: &Path, deck_id: &str, fingerprint: u64) -> Vec<String> {
    let mut bases: Vec<String> = personal_cards_with_content(deck_path, deck_id, fingerprint)
        .iter()
        .filter_map(|card| card.token.as_deref())
        .map(|token| crate::token::card_id(token, None, false, None))
        .collect();
    bases.dedup();
    bases
}

fn personal_cards_with_content(deck_path: &Path, deck_id: &str, fingerprint: u64) -> Vec<Card> {
    let Ok(text) = std::fs::read_to_string(crate::personal::sidecar_path(deck_path)) else {
        return Vec::new();
    };
    let Some(cards) = sidecar_cards(deck_id, &text) else {
        return Vec::new();
    };
    cards
        .into_iter()
        .filter(|card| card.block_fingerprint == fingerprint)
        .collect()
}

fn stamp_generated_regions(block: &str) -> AnyResult<String> {
    let deck_id = crate::token::mint().map_err(|e| anyhow::anyhow!("cannot mint a token: {e}"))?;
    // A scratch header so the non-initializing stamper accepts the file;
    // stripped after, so only the block's own bytes are appended.
    let head = format!("---\nid: \"deck-{deck_id}\"\n---\n\n");
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("remediation.md");
    std::fs::write(&path, format!("{head}{block}"))?;
    crate::stamp::stamp_initialized_deck(&path)?;
    let stamped = std::fs::read_to_string(&path)?;
    stamped
        .strip_prefix(&head)
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("the stamper rewrote the scratch header"))
}

fn bind_block_to_topic(block: &str, token: &str, topic_id: &str) -> Option<String> {
    let id_line = format!("<!-- id: {token} -->");
    let at = block.find(&id_line)?;
    Some(format!(
        "{}{}{}",
        &block[..at],
        topic_line(topic_id),
        &block[at..]
    ))
}

fn stamp_block(block: &str, token: &str) -> String {
    match block.strip_suffix('\n') {
        Some(body) => format!("{body}\n<!-- id: {token} -->\n"),
        None => format!("{block}\n<!-- id: {token} -->"),
    }
}

// Syncthing's own naming convention for conflict copies: `<stem>.sync-conflict-*.<ext>`.
pub fn sync_conflicts(store_path: &Path) -> Vec<PathBuf> {
    let direct = store_path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "progress")
        && store_path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|name| name == ".alix")
        && store_path
            .extension()
            .is_some_and(|extension| extension == "json");
    let mut out = if direct {
        conflict_copies(store_path)
    } else {
        let progress = if store_path
            .file_name()
            .is_some_and(|name| name == "progress")
            && store_path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == ".alix")
        {
            store_path.to_path_buf()
        } else {
            crate::state::UserFiles::new(store_path).progress()
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
                    .is_some_and(|name| {
                        !crate::workspace::is_private_name(name)
                            && crate::workspace::is_conflict_name(name)
                            && !crate::workspace::is_backup_name(name)
                    })
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

    fn empty_deck(dir: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
        let deck = dir.path().join(name);
        std::fs::write(&deck, "").unwrap();
        deck
    }

    fn sync_document(deck_id: &str, revision: u64, device: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "deck_id": deck_id,
            "subject": "deck.md",
            "revision": revision,
            "cards": {},
            "writer": { "device": device, "at_ms": 7 }
        }))
        .unwrap()
    }

    #[test]
    fn sync_push_revision_law_preserves_conflicts_and_backs_up_accepts() {
        let cases = [
            ("none + absent", None, None, true, 1),
            ("moved + present", Some(3), Some(2), false, 3),
            ("none + present", Some(3), None, false, 3),
            ("equal + present", Some(3), Some(3), true, 4),
        ];
        for (name, desktop, pulled, accepted, expected_revision) in cases {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("deck-sync.json");
            if let Some(revision) = desktop {
                std::fs::write(&path, sync_document("deck-sync", revision, "desktop")).unwrap();
            }
            let before = std::fs::read(&path).ok();
            let document =
                ValidatedDeckDocument::parse(&sync_document("deck-sync", 99, "phone"), "deck-sync")
                    .unwrap();

            let outcome = sync_push_document(&path, "deck-sync", pulled, document).unwrap();

            match outcome {
                SyncPushOutcome::Accepted { revision } => {
                    assert!(accepted, "{name}: unexpected accept");
                    assert_eq!(expected_revision, revision, "{name}: committed revision");
                    let (disk, _, data) = read_deck_data(&path, "deck-sync", None).unwrap();
                    assert_eq!(revision, disk, "{name}: disk revision");
                    assert_eq!(
                        Some("phone"),
                        data.writer.as_ref().map(|w| w.device.as_str()),
                        "{name}: body writer survives"
                    );
                    let backup = path.with_extension("json.bak");
                    if let Some(before) = before {
                        let backup = std::fs::read(&backup).unwrap_or_else(|error| {
                            panic!("{name}: accepted push must write its backup: {error}")
                        });
                        assert_eq!(
                            before, backup,
                            "{name}: backup is the exact previous document"
                        );
                    } else {
                        assert!(!backup.exists(), "{name}: first push creates no backup");
                    }
                }
                SyncPushOutcome::Conflict {
                    desktop_revision,
                    pulled_revision,
                    ..
                } => {
                    assert!(!accepted, "{name}: unexpected conflict");
                    assert_eq!(desktop, desktop_revision, "{name}: desktop revision");
                    assert_eq!(pulled, pulled_revision, "{name}: pulled revision");
                    assert_eq!(
                        before,
                        std::fs::read(&path).ok(),
                        "{name}: conflict leaves disk unchanged"
                    );
                    assert!(
                        !path.with_extension("json.bak").exists(),
                        "{name}: conflict creates no backup"
                    );
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn sync_push_only_treats_not_found_as_an_absent_document() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck-sync.json");
        let original = sync_document("deck-sync", 3, "desktop");
        std::fs::write(&path, &original).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let document =
            ValidatedDeckDocument::parse(&sync_document("deck-sync", 99, "phone"), "deck-sync")
                .unwrap();

        let error = sync_push_document(&path, "deck-sync", None, document).unwrap_err();

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let StoreError::Io {
            path: error_path,
            source,
        } = error
        else {
            panic!("a non-NotFound read error must be returned unchanged: {error}");
        };
        assert_eq!(path, error_path);
        assert_eq!(std::io::ErrorKind::PermissionDenied, source.kind());
        assert_eq!(original, std::fs::read(&path).unwrap());
        assert!(!path.with_extension("json.bak").exists());
    }

    #[test]
    fn open_creates_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        let store = Store::open(&path).unwrap();
        assert!(store.is_empty());
    }

    #[test]
    fn orphans_are_the_keys_with_no_live_card_or_deck_and_prune_clears_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert("live").introduced_ms = Some(0);
        store.get_or_insert("gone").introduced_ms = Some(0);
        store.get_or_insert("card-vq").introduced_ms = Some(0);
        store.set_last_launch("d1", Launch::Recall);
        store.set_last_launch("d2", Launch::Recall);

        for topic_id in ["topic-live", "topic-gone"] {
            let record = SittingRecord {
                topics: vec![TopicRecord {
                    topic_id: topic_id.to_string(),
                    fingerprint: "f".to_string(),
                    pass: true,
                }],
                ..Default::default()
            };
            store.record_sitting("d1", &[], &record, 1, false, &ExamStamp::default());
        }

        let known_cards: HashSet<String> = ["live".to_string(), "card-vq".to_string()]
            .into_iter()
            .collect();
        let known_deck_ids: HashSet<String> = ["d1".to_string()].into_iter().collect();
        let known_topic_ids: HashMap<String, HashSet<String>> =
            HashMap::from([("d1".to_string(), HashSet::from(["topic-live".to_string()]))]);
        let orphans = store.orphans(&known_cards, &known_deck_ids, &known_topic_ids);
        assert_eq!(vec!["gone".to_string()], orphans.cards);
        assert_eq!(vec!["d2".to_string()], orphans.decks);
        assert_eq!(
            vec![("d1".to_string(), "topic-gone".to_string())],
            orphans.topics,
            "a topic result under a known deck that names no topic of it"
        );
        assert_eq!(3, orphans.len());

        assert_eq!(3, store.prune_orphans(&orphans));
        assert!(store.get("live").is_some());
        assert!(store.get("card-vq").is_some());
        assert_eq!(Some(Launch::Recall), store.last_launch("d1"));
        assert!(store.get("gone").is_none());
        assert_eq!(None, store.last_launch("d2"));
        assert!(
            store.topic_result("d1", "topic-live").is_some(),
            "the live result stays"
        );
        assert_eq!(None, store.topic_result("d1", "topic-gone"));
    }

    #[test]
    fn wipe_deck_clears_every_family_for_its_tokens_and_spares_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        store.get_or_insert("card-doom").introduced_ms = Some(0);
        store.get_or_insert("card-doom-ba1b2c3").introduced_ms = Some(0);
        store.set_deck_passed("doomed", 1, &ExamStamp::default());
        store.get_or_insert("keep").introduced_ms = Some(0);
        store.set_deck_passed("keep", 1, &ExamStamp::default());

        let tokens: HashSet<String> = ["card-doom".to_string()].into_iter().collect();
        let wiped = store.wipe_deck(&tokens, "doomed");

        assert_eq!(2, wiped, "the base and the span schedule both count");
        assert!(store.get("card-doom").is_none());
        assert!(store.get("card-doom-ba1b2c3").is_none());
        assert!(!store.deck_passed("doomed"));
        assert!(store.get("keep").is_some());
        assert!(store.deck_passed("keep"));
    }

    #[test]
    fn a_stale_positional_key_survives_wipe_and_lists_as_an_orphan() {
        // A `-N` key is not part of the id grammar, so token selection cannot
        // claim it; the raw string comparison in `orphans` still surfaces it
        // for `reset --orphans`.
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert("card-doom-0").introduced_ms = Some(0);
        let tokens: HashSet<String> = ["card-doom".to_string()].into_iter().collect();
        assert_eq!(0, store.wipe_deck(&tokens, "doomed"));
        let orphans = store.orphans(&HashSet::new(), &HashSet::new(), &HashMap::new());
        assert_eq!(vec!["card-doom-0".to_string()], orphans.cards);
    }

    #[test]
    fn save_stamps_the_writer_and_a_reopen_sees_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        let mut store = Store::open(&path).unwrap();
        store.device = Some("desk-1".into());
        store.save().unwrap();

        let reopened = Store::open(&path).unwrap();
        assert_eq!("desk-1", reopened.last_writer.unwrap().device);
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
        assert_eq!("desk-1", reopened.last_writer.unwrap().device);
    }

    #[test]
    fn sync_conflicts_finds_syncthing_copies_and_ignores_near_misses() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".alix/progress")).unwrap();
        let store_path = dir.path().join(".alix/progress/deck1.json");
        std::fs::write(&store_path, "{}").unwrap();
        let conflict = dir
            .path()
            .join(".alix/progress/deck1.sync-conflict-20260714-101112-ABCDEF7.json");
        std::fs::write(&conflict, "{}").unwrap();
        for near_miss in [
            ".alix/progress/recent.sync-conflict-20260714-101112-AAAAAAA.json",
            ".alix/progress/deck1.sync-conflict-20260714.txt",
            ".alix/progress/deck1.json.tmp",
        ] {
            std::fs::write(dir.path().join(near_miss), "{}").unwrap();
        }
        assert_eq!(sync_conflicts(&store_path), vec![conflict]);
        assert_eq!(
            sync_conflicts(&dir.path().join("missing/progress/deck1.json")),
            Vec::<PathBuf>::new()
        );

        let wrong_extension = dir.path().join(".alix/progress/deck2.txt");
        std::fs::write(&wrong_extension, "{}").unwrap();
        std::fs::write(
            dir.path()
                .join(".alix/progress/deck2.sync-conflict-20260714-phone.txt"),
            "{}",
        )
        .unwrap();
        assert!(sync_conflicts(&wrong_extension).is_empty());

        std::fs::create_dir(dir.path().join("elsewhere")).unwrap();
        let wrong_parent = dir.path().join("elsewhere/deck3.json");
        std::fs::write(&wrong_parent, "{}").unwrap();
        std::fs::write(
            dir.path()
                .join("elsewhere/deck3.sync-conflict-20260714-phone.json"),
            "{}",
        )
        .unwrap();
        assert!(sync_conflicts(&wrong_parent).is_empty());
    }

    #[test]
    fn sync_conflicts_accepts_the_canonical_progress_directory_it_scans() {
        let dir = tempfile::tempdir().unwrap();
        let progress = dir.path().join(".alix/progress");
        std::fs::create_dir_all(&progress).unwrap();
        let conflict = progress.join("deck1.sync-conflict-20260714-phone.json");
        std::fs::write(&conflict, "{}").unwrap();

        assert_eq!(vec![conflict], sync_conflicts(&progress));
    }

    #[test]
    fn sync_conflicts_does_not_scan_progress_outside_dot_alix() {
        let dir = tempfile::tempdir().unwrap();
        let progress = dir.path().join("elsewhere/progress");
        std::fs::create_dir_all(&progress).unwrap();
        std::fs::write(
            progress.join("deck1.sync-conflict-20260714-phone.json"),
            "{}",
        )
        .unwrap();

        assert!(sync_conflicts(&progress).is_empty());
    }

    #[test]
    fn sync_conflicts_does_not_scan_a_non_progress_directory_under_dot_alix() {
        let dir = tempfile::tempdir().unwrap();
        let not_progress = dir.path().join(".alix/not-progress");
        std::fs::create_dir_all(&not_progress).unwrap();
        std::fs::write(
            not_progress.join("deck1.sync-conflict-20260714-phone.json"),
            "{}",
        )
        .unwrap();

        assert!(sync_conflicts(&not_progress).is_empty());
    }

    #[test]
    #[cfg(feature = "full")]
    fn progress_stamp_ignores_every_non_document_entry() {
        let dir = tempfile::tempdir().unwrap();
        let baseline = progress_dir_stamp(dir.path());

        std::fs::create_dir(dir.path().join("folder.json")).unwrap();
        assert_eq!(baseline, progress_dir_stamp(dir.path()));

        std::fs::write(dir.path().join("notes.txt"), "not progress").unwrap();
        assert_eq!(baseline, progress_dir_stamp(dir.path()));

        std::fs::write(
            dir.path().join("deck.sync-conflict-20260714-phone.json"),
            "{}",
        )
        .unwrap();
        assert_eq!(baseline, progress_dir_stamp(dir.path()));

        std::fs::write(dir.path().join("deck.json"), "{}").unwrap();
        assert_ne!(baseline, progress_dir_stamp(dir.path()));
    }

    #[test]
    fn sync_conflicts_finds_per_deck_progress_copies() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".alix/progress")).unwrap();
        let progress = dir
            .path()
            .join(".alix/progress/deck1.sync-conflict-20260714-phone.json");
        std::fs::write(&progress, "{}").unwrap();
        std::fs::write(dir.path().join(".alix/progress/deck1.json.bak"), "{}").unwrap();

        assert_eq!(
            sync_conflicts(dir.path()),
            vec![progress],
            "the backup an accepted push leaves is not a conflict copy"
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

    // `directories` reads a Windows Known Folder, not the environment, so
    // there is no data home for this pair to configure there: CI observed
    // `AppData\Roaming\alix\data` with both HOME and XDG_DATA_HOME pointed
    // at a temp dir.
    #[cfg(unix)]
    #[test]
    fn process_data_paths_child() {
        let Some(root) = std::env::var_os("ALIX_STORE_PATH_CHILD") else {
            return;
        };
        let root = PathBuf::from(root);
        let path = directories::ProjectDirs::from("", "", "alix")
            .expect("the process yields project directories")
            .data_dir()
            .to_path_buf();
        // The layout BETWEEN them is the platform's, not ours: macOS resolves
        // a data home to `Library/Application Support`, so asserting
        // `root/alix` would pass only on Linux.
        assert!(
            path.starts_with(&root),
            "the configured data home roots the store path: {path:?} not under {root:?}"
        );
        assert!(
            path.ends_with("alix"),
            "and alix keeps its own directory there: {path:?}"
        );

        let label = device_label().expect("the process data directory yields a device label");
        assert!(
            label.starts_with("alix-") && label.len() == 9,
            "generated label shape: {label:?}"
        );
        assert!(path.join("device").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn process_data_paths_use_the_configured_data_home() {
        #[cfg(all(unix, feature = "full"))]
        let _lock = crate::testutil::exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "store::tests::process_data_paths_child",
                "--nocapture",
            ])
            .env("ALIX_STORE_PATH_CHILD", dir.path())
            .env("XDG_DATA_HOME", dir.path())
            .env("HOME", dir.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
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
        store.get_or_insert("not-a-token").introduced_ms = Some(0);
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
            .get_or_insert("1")
            .record_review(100, Grade::Pass, Depth::Recall, false);
        store
            .get_or_insert("2")
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
        let state = store.get_or_insert("42");
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
                propagated: false,
                confidence: None,
            }],
            state.history
        );
    }

    #[test]
    fn deck_document_roundtrip_records_owner_and_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress/deck1.json");
        let mut store = Store::open_deck(&path, "deck1", "old.md").unwrap();
        store.get_or_insert("card1").introduced_ms = Some(1);
        store.set_deck_passed("deck1", 2, &ExamStamp::default());
        store.save().unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\": 1"));
        assert!(text.contains("\"deck_id\": \"deck1\""));
        assert!(text.contains("\"revision\": 1"));

        let reopened = Store::open_deck(&path, "deck1", "old.md").unwrap();
        assert!(reopened.get("card1").is_some());
        assert!(reopened.deck_passed("deck1"));
    }

    #[test]
    fn opening_a_deck_document_under_a_new_filename_keeps_its_deck_level_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress/deck1.json");
        let mut store = Store::open_deck(&path, "deck1", "old.md").unwrap();
        store.set_deck_passed("deck1", 2, &ExamStamp::default());
        store.get_or_insert("card-one").introduced_ms = Some(1);
        store.save().unwrap();

        // Deck-level state follows the id, so a reopen under a new filename
        // (same document, same deck_id) still finds it: the subject argument
        // no longer rebinds anything.
        let renamed = Store::open_deck(&path, "deck1", "new.md").unwrap();
        assert!(renamed.deck_passed("deck1"));
        assert!(renamed.get("card-one").is_some());
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
        first.get_or_insert("newer").introduced_ms = Some(1);
        first.save().unwrap();
        stale.get_or_insert("stale").introduced_ms = Some(1);

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
        fresh.get_or_insert("card1").introduced_ms = Some(1);
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
        let state = store.get_or_insert("42");
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
    fn deck_passed_roundtrips_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");

        let mut store = Store::open(&path).unwrap();
        assert!(!store.deck_passed("deck1"));
        assert_eq!(None, store.deck_passed_at("deck1"));
        store.set_deck_passed("deck1", 1234, &ExamStamp::default());
        assert!(store.deck_passed("deck1"));
        assert_eq!(Some(1234), store.deck_passed_at("deck1"));
        store.save().unwrap();

        let mut reloaded = Store::open(&path).unwrap();
        assert!(reloaded.deck_passed("deck1"));
        assert_eq!(Some(1234), reloaded.deck_passed_at("deck1"));
        assert!(reloaded.remove_deck_progress("deck1"));
        assert!(!reloaded.deck_passed("deck1"));
        assert!(!reloaded.remove_deck_progress("deck1"));
    }

    #[test]
    fn clear_also_drops_deck_passed() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.set_deck_passed("a", 1, &ExamStamp::default());
        store.clear();
        assert!(!store.deck_passed("a"));
    }

    #[test]
    fn remove_drops_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert("42").introduced_ms = Some(1000);
        assert!(store.remove("42"));
        assert!(store.get("42").is_none());
        assert!(!store.remove("42"));
    }

    #[test]
    fn clear_empties_and_counts() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert("1").introduced_ms = Some(0);
        store.get_or_insert("2").introduced_ms = Some(0);
        assert_eq!(2, store.clear());
        assert!(store.is_empty());
        assert_eq!(0, store.clear());
    }

    #[test]
    fn history_is_capped() {
        let mut state = CardState::new();
        for i in 0..(HISTORY_CAP as u64 + 10) {
            state.record_review(i, Grade::Pass, Depth::Recall, false);
        }
        assert_eq!(HISTORY_CAP, state.history.len());
        assert_eq!(10, state.history[0].ts_ms);
        assert_eq!(HISTORY_CAP as u32 + 10, state.total_reviews);
    }

    #[test]
    fn streak_resets_on_fail() {
        let mut state = CardState::new();
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
        let mut state = CardState::new();
        state.record_review(10, Grade::Partial, Depth::Recall, false);
        assert_eq!(Grade::Partial, state.history.last().unwrap().grade);
        assert_eq!(1, state.total_reviews);
        assert_eq!(1, state.total_passes);
        assert_eq!(1, state.streak);
    }

    #[test]
    fn recall_and_reconstruct_schedules_are_independent() {
        let mut s = CardState::new();
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
            "no cross-crediting: recognize starts empty"
        );
    }

    #[test]
    fn per_depth_schedules_survive_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("deck1.json")).unwrap();
        let st = store.get_or_insert("7");
        *st.schedule_slot(Depth::Reconstruct).unwrap() = Some(FsrsState {
            stability: 4.5,
            ..Default::default()
        });
        *st.schedule_slot(Depth::Recognize).unwrap() = Some(FsrsState {
            stability: 2.5,
            ..Default::default()
        });
        st.record_review(2_000, Grade::Pass, Depth::Reconstruct, false);
        store.save().unwrap();
        let reloaded = Store::open(dir.path().join("deck1.json")).unwrap();
        let st = reloaded.get("7").unwrap();
        assert_eq!(
            Some(4.5),
            st.schedule(Depth::Reconstruct).map(|f| f.stability)
        );
        assert_eq!(
            Some(2.5),
            st.schedule(Depth::Recognize).map(|f| f.stability)
        );
        assert_eq!(Depth::Reconstruct, st.history[0].depth);
    }

    #[test]
    fn a_walk_grade_serializes_as_the_grade_name_and_is_absent_when_unset() {
        let state = CardState {
            walk_grade: Some(Grade::Partial),
            ..CardState::new()
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains(r#""walk_grade":"Partial""#), "set: {json}");
        assert_eq!(
            state,
            serde_json::from_str::<CardState>(&json).unwrap(),
            "round trip"
        );
        let unset = serde_json::to_string(&CardState::new()).unwrap();
        assert!(!unset.contains("walk_grade"), "unset: {unset}");
    }

    #[test]
    fn history_grades_survive_save_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        let mut store = Store::open(&path).unwrap();
        let st = store.get_or_insert("7");
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
        store.get_or_insert("9").recall = Some(FsrsState {
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
            "## a\n1\n<!-- id: card-q1 -->\n\n## b\n2\n<!-- id: card-q2 -->\n",
        )
        .unwrap()
    }

    #[test]
    fn a_deck_with_all_mature_recall_cards_is_recall_solid() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let cards = two_cards();
        for card in &cards {
            store.get_or_insert(&card.id().unwrap()).recall = Some(FsrsState {
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
            store.get_or_insert(&card.id().unwrap()).recall = Some(FsrsState {
                stability: 30.0,
                ..Default::default()
            });
        }
        record_badges(&mut store, "t", &cards, 1_000);
        assert_eq!(Some(1_000), store.badge_earned("t", Depth::Recall));

        store.get_or_insert(&cards[0].id().unwrap()).recall = Some(FsrsState {
            stability: 3.0,
            ..Default::default()
        });

        assert!(!badge_solid(&cards, &store, Depth::Recall));
        assert_eq!(Some(1_000), store.badge_earned("t", Depth::Recall));
    }

    #[test]
    fn recognize_badge_needs_every_card_mature_at_recognize() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let cards = two_cards();
        store.get_or_insert(&cards[0].id().unwrap()).recognize = Some(FsrsState {
            stability: 30.0,
            ..Default::default()
        });
        assert!(
            !badge_solid(&cards, &store, Depth::Recognize),
            "second card not yet mature at recognize"
        );

        store.get_or_insert(&cards[1].id().unwrap()).recognize = Some(FsrsState {
            stability: 30.0,
            ..Default::default()
        });
        assert!(badge_solid(&cards, &store, Depth::Recognize));
    }

    #[test]
    fn every_last_launch_roundtrips_through_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.json");
        let mut store = Store::open(&path).unwrap();
        assert_eq!(None, store.last_launch("p"));
        for launch in [
            Launch::Recognize,
            Launch::Recall,
            Launch::Reconstruct,
            Launch::Walk,
        ] {
            store.set_last_launch("p", launch);
            store.save().unwrap();
            let reloaded = Store::open(&path).unwrap();
            assert_eq!(Some(launch), reloaded.last_launch("p"), "{launch:?}");
        }
    }

    #[test]
    fn remediation_and_tutor_drafts_refuse_a_duplicated_topic_id() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "d.md");
        std::fs::write(
            &deck,
            "---\nid: \"deck-d\"\n---\n# A\n<!-- id: topic-a -->\n## q\na\n<!-- id: card-q1 -->\n\
             # B\n<!-- id: topic-a -->\n## r\nb\n<!-- id: card-r1 -->\n",
        )
        .unwrap();
        let sidecar = crate::personal::sidecar_path(&deck);
        let before = "---\nfor: deck-d\n---\n";
        std::fs::write(&sidecar, before).unwrap();
        let finding = crate::topic::duplicated_topics(&Deck::load(&deck).unwrap(), None)
            .into_iter()
            .find(|finding| finding.id == "topic-a")
            .expect("the fixture repeats topic-a")
            .to_string();
        assert!(
            finding.contains(&format!("{}:", deck.display())),
            "the finding names the copy: {finding}"
        );

        let tutor = mint_tutor_card(
            &mut store,
            &deck,
            "deck-d",
            "topic-a",
            "capital?",
            &["Paris".to_string()],
            &HashSet::new(),
        )
        .unwrap_err();
        assert_eq!(finding, tutor.to_string(), "tutor draft");
        let remediation = store_remediation_cards(
            &mut store,
            Some(&deck),
            "deck-d",
            "topic-a",
            &HashSet::new(),
            "## Why?\nbecause\n",
            None,
        )
        .into_result()
        .unwrap_err();
        assert_eq!(finding, remediation.to_string(), "remediation");
        let remote = mint_or_return_kept_card(
            &deck,
            "deck-d",
            "topic-a",
            "capital?",
            &["Paris".to_string()],
            &HashSet::new(),
        )
        .map(|minted| minted.id);
        assert_eq!(
            Err(finding.clone()),
            remote.map_err(|error| error.to_string()),
            "remote draft"
        );
        assert_eq!(
            before,
            std::fs::read_to_string(&sidecar).unwrap(),
            "nothing is written"
        );
    }

    #[test]
    fn an_existing_sidecar_card_with_a_topic_line_is_still_found_by_every_sidecar_reader() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "geo.md");
        let sidecar = crate::personal::sidecar_path(&deck);
        let text = "---\nfor: geo.md\n---\n\n## capital of spain?\nMadrid\n\
                    <!-- topic: topic-t -->\n<!-- id: card-6v3c7x4k1m8q3z5t0b2n4d8f9w -->\n\n\
                    <!-- note: card-6v3c7x4k1m8q3z5t0b2n4d8f9w -->\n> [!NOTE]\n> mine\n";
        std::fs::write(&sidecar, text).unwrap();
        let spain = || ["Madrid".to_string()];

        let minted = mint_tutor_card(
            &mut store,
            &deck,
            "geo.md",
            "topic-t",
            "capital of spain?",
            &spain(),
            &HashSet::new(),
        );
        assert!(
            matches!(minted, Err(MintError::Duplicate)),
            "tutor dedup: {minted:?}"
        );
        let kept = mint_or_return_kept_card(
            &deck,
            "geo.md",
            "topic-t",
            "capital of spain?",
            &spain(),
            &HashSet::new(),
        )
        .map(|kept| kept.id);
        assert_eq!(
            "card-6v3c7x4k1m8q3z5t0b2n4d8f9w",
            kept.unwrap_or_default(),
            "kept card"
        );
        let recorded = record_minted_card(
            &mut store,
            &deck,
            "geo.md",
            "## capital of spain?\nMadrid\n<!-- topic: topic-t -->\n<!-- id: card-6v3c7x4k1m8q3z5t0b2n4d8f9x -->\n",
            &HashSet::new(),
        );
        assert!(
            matches!(recorded, Err(MintError::Duplicate)),
            "recorded dedup: {recorded:?}"
        );
        let stored = store_remediation(
            &mut store,
            &deck,
            "geo.md",
            "## capital of spain?\nMadrid\n",
            None,
        )
        .unwrap();
        assert_eq!(0, stored, "remediation dedup");
        assert_eq!(
            text,
            std::fs::read_to_string(&sidecar).unwrap(),
            "nothing written"
        );
    }

    #[test]
    fn a_personal_card_without_a_topic_line_still_dedups_a_new_card() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "geo.md");
        std::fs::write(&deck, "").unwrap();
        let sidecar = crate::personal::sidecar_path(&deck);
        let text = "---\nfor: geo.md\n---\n\n## capital of spain?\nMadrid\n\
                    <!-- id: card-6v3c7x4k1m8q3z5t0b2n4d8f9w -->\n";
        std::fs::write(&sidecar, text).unwrap();

        let minted = mint_tutor_card(
            &mut store,
            &deck,
            "geo.md",
            "topic-t",
            "capital of spain?",
            &["Madrid".to_string()],
            &HashSet::new(),
        );
        assert!(
            matches!(minted, Err(MintError::Duplicate)),
            "tutor: {minted:?}"
        );
        let stored = store_remediation(
            &mut store,
            &deck,
            "geo.md",
            "## capital of spain?\nMadrid\n",
            None,
        )
        .unwrap();
        assert_eq!(0, stored, "remediation");
        assert_eq!(
            text,
            std::fs::read_to_string(&sidecar).unwrap(),
            "nothing written"
        );
    }

    #[test]
    fn minting_and_remediation_fail_with_the_load_error_when_the_deck_cannot_load() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = dir.path().join("broken.md");
        std::fs::write(&deck, "## q\n").unwrap();
        let load_error = Deck::load(&deck).unwrap_err().to_string();

        let tutor = mint_tutor_card(
            &mut store,
            &deck,
            "deck-b",
            "deck-b",
            "capital?",
            &["Paris".to_string()],
            &HashSet::new(),
        )
        .unwrap_err();
        assert_eq!(load_error, tutor.to_string(), "tutor draft");
        let remediation = store_remediation_cards(
            &mut store,
            Some(&deck),
            "deck-b",
            "deck-b",
            &HashSet::new(),
            "## Why?\nbecause\n",
            None,
        )
        .into_result()
        .unwrap_err();
        assert_eq!(load_error, format!("{remediation:#}"), "remediation");
        assert!(
            !crate::personal::sidecar_path(&deck).exists(),
            "nothing is written"
        );
    }

    #[test]
    fn a_revived_remediation_card_takes_the_topic_of_the_call_that_revived_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "d.md");
        std::fs::write(&deck, "").unwrap();
        let text = "## Why does X happen?\nbecause of Y\n";
        let cap = Some(30u32);
        let store_for = |store: &mut Store, topic: &str| {
            store_remediation_cards(
                store,
                Some(&deck),
                "d.md",
                topic,
                &std::collections::HashSet::new(),
                text,
                cap,
            )
            .into_result()
            .unwrap()
        };
        assert_eq!(1, store_for(&mut store, "topic-a"), "created under topic-a");
        let id = sidecar_ids(&deck, "d.md")[0].clone();
        store.get_or_insert(&id).recall = Some(FsrsState {
            state: 2,
            scheduled_days: 90,
            ..Default::default()
        });

        assert_eq!(1, store_for(&mut store, "topic-b"), "revived by topic-b");

        let sidecar = std::fs::read_to_string(crate::personal::sidecar_path(&deck)).unwrap();
        assert!(
            sidecar.contains(&format!("<!-- topic: topic-b -->\n<!-- id: {id} -->")),
            "the revived card's topic line names topic-b:\n{sidecar}"
        );
        assert_eq!(
            1,
            sidecar.matches("<!-- topic: ").count(),
            "the line is rewritten, not added:\n{sidecar}"
        );
    }

    #[test]
    fn mint_tutor_card_writes_the_card_into_the_sidecar() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "geo.md");
        let id = mint_tutor_card(
            &mut store,
            &deck,
            "geo.md",
            "topic-t",
            "capital of france",
            &["Paris".to_string()],
            &HashSet::new(),
        )
        .unwrap();
        let text =
            std::fs::read_to_string(crate::personal::sidecar_path(&deck)).expect("a sidecar");
        assert!(
            text.contains(&id),
            "the sidecar carries the minted card: {text}"
        );
        assert!(
            store.get(&id).is_some(),
            "and the store holds an empty entry for it"
        );
        assert!(
            store.progress(&id).is_none(),
            "a tutor-written card is new: the drill introduces it"
        );
    }

    #[test]
    fn a_schedule_entry_exists_whenever_a_card_is_minted() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("s.json")).unwrap();
        let geo = empty_deck(&dir, "geo.md");
        let tutor = mint_tutor_card(
            &mut store,
            &geo,
            "geo.md",
            "topic-t",
            "capital of italy?",
            &["Rome".to_string()],
            &HashSet::new(),
        )
        .unwrap();
        assert!(store.get(&tutor).is_some(), "the schedule entry exists");

        let deck = empty_deck(&dir, "d.md");
        store_remediation(
            &mut store,
            &deck,
            "d.md",
            "## Why does X happen?\nbecause Y\n",
            None,
        )
        .unwrap();
        let gap = sidecar_ids(&deck, "d.md")[0].clone();
        assert!(store.get(&gap).is_some());
    }

    #[test]
    fn a_double_tutor_mint_reports_duplicate() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "geo.md");
        let empty = HashSet::new();
        mint_tutor_card(
            &mut store,
            &deck,
            "geo.md",
            "topic-t",
            "capital of spain?",
            &["Madrid".to_string()],
            &empty,
        )
        .unwrap();
        let err = mint_tutor_card(
            &mut store,
            &deck,
            "geo.md",
            "topic-t",
            "capital of spain?",
            &["Madrid".to_string()],
            &empty,
        )
        .unwrap_err();
        assert!(matches!(err, MintError::Duplicate));
    }

    #[test]
    fn mint_tutor_card_rejects_content_already_in_the_deck() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "geo.md");
        let authored = crate::parser::parse_str(
            "geo.md",
            "## capital of france?\nParis\n<!-- id: card-france -->\n",
        )
        .unwrap();
        let deck_fingerprints = HashSet::from([authored[0].block_fingerprint]);

        let err = mint_tutor_card(
            &mut store,
            &deck,
            "geo.md",
            "topic-t",
            "capital of france?",
            &["Paris".to_string()],
            &deck_fingerprints,
        )
        .unwrap_err();

        assert!(matches!(err, MintError::Duplicate));
        assert!(
            !crate::personal::sidecar_path(&deck).exists(),
            "a card duplicated from the authored deck must not reach the sidecar"
        );
    }

    #[test]
    fn mint_tutor_card_rejects_either_empty_side_before_parsing() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "geo.md");
        for (front, back) in [
            ("  ", vec!["Paris".to_string()]),
            ("capital?", vec!["  ".to_string()]),
        ] {
            let err = mint_tutor_card(
                &mut store,
                &deck,
                "geo.md",
                "topic-t",
                front,
                &back,
                &HashSet::new(),
            )
            .unwrap_err();
            let MintError::Malformed(message) = err else {
                panic!("an empty side must be malformed: {err:?}");
            };
            assert_eq!("front and back must both be non-empty", message);
        }
    }

    #[test]
    fn mint_tutor_card_rejects_an_embedded_newline() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "geo.md");
        let err = mint_tutor_card(
            &mut store,
            &deck,
            "geo.md",
            "topic-t",
            "capital?",
            &["Paris\n% direction: reverse".to_string()],
            &HashSet::new(),
        )
        .unwrap_err();
        assert!(matches!(err, MintError::Malformed(_)));
    }

    #[test]
    fn mint_personal_card_hands_back_the_block_it_appended() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let deck = empty_deck(&dir, "geo.md");
        let minted = mint_personal_card(
            &deck,
            "geo.md",
            "topic-t",
            "capital of spain?",
            &["Madrid".to_string()],
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(
            format!(
                "## capital of spain?\nMadrid\n<!-- topic: topic-t -->\n<!-- id: {} -->\n",
                minted.id
            ),
            minted.block,
            "the block is the stamped card text"
        );
        let text =
            std::fs::read_to_string(crate::personal::sidecar_path(&deck)).expect("a sidecar");
        assert!(
            text.ends_with(&minted.block),
            "the sidecar ends with that exact block:\n{text}"
        );
    }

    #[test]
    fn a_retried_mint_returns_the_kept_card_only_when_front_and_back_are_identical() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let deck = empty_deck(&dir, "geo.md");
        let authored = crate::parser::parse_str(
            "geo.md",
            "## capital of france?\nParis\n<!-- id: card-france -->\n",
        )
        .unwrap();
        let deck_fingerprints = HashSet::from([authored[0].block_fingerprint]);
        let mint = |front: &str, back: &str| {
            mint_or_return_kept_card(
                &deck,
                "geo.md",
                "topic-t",
                front,
                &[back.to_string()],
                &deck_fingerprints,
            )
        };

        let kept = mint("capital of **spain**?", "Madrid").unwrap();
        let retried = mint("  capital of **spain**?  ", "Madrid").unwrap();
        assert_eq!(
            (&kept.id, &kept.block),
            (&retried.id, &retried.block),
            "identical retry: the kept card comes back"
        );
        let text = std::fs::read_to_string(crate::personal::sidecar_path(&deck)).unwrap();
        assert_eq!(
            1,
            text.matches("<!-- id: ").count(),
            "identical retry appends nothing:\n{text}"
        );

        assert!(
            matches!(
                mint("capital of spain?", "Madrid"),
                Err(MintError::Duplicate)
            ),
            "same fingerprint, different text: still a duplicate"
        );
        assert!(
            matches!(
                mint("capital of france?", "Paris"),
                Err(MintError::Duplicate)
            ),
            "a deck card is never handed back as a kept card"
        );
    }

    #[test]
    fn a_retried_mint_does_not_rebuild_a_directive_card_as_plain_text() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let deck = empty_deck(&dir, "geo.md");
        let sidecar = crate::personal::sidecar_path(&deck);
        let kept = "---\nfor: geo.md\n---\n\n## capital of spain?\nMadrid\n<!-- input: draw -->\n<!-- topic: topic-t -->\n<!-- id: card-spain -->\n";
        std::fs::write(&sidecar, kept).unwrap();

        let retried = mint_or_return_kept_card(
            &deck,
            "geo.md",
            "topic-t",
            "capital of spain?",
            &["Madrid".to_string()],
            &HashSet::new(),
        );

        assert!(
            matches!(retried, Err(MintError::Duplicate)),
            "different sidecar text must stay a refused duplicate"
        );
        assert_eq!(kept, std::fs::read_to_string(sidecar).unwrap());
    }

    #[test]
    fn an_authored_duplicate_is_not_returned_from_a_matching_sidecar_card() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let deck = empty_deck(&dir, "geo.md");
        let authored = crate::parser::parse_str(
            "geo.md",
            "## capital of spain?\nMadrid\n<!-- id: card-authored -->\n",
        )
        .unwrap();
        std::fs::write(
            crate::personal::sidecar_path(&deck),
            "---\nfor: geo.md\n---\n\n## capital of spain?\nMadrid\n<!-- topic: topic-t -->\n<!-- id: card-personal -->\n",
        )
        .unwrap();

        let retried = mint_or_return_kept_card(
            &deck,
            "geo.md",
            "topic-t",
            "capital of spain?",
            &["Madrid".to_string()],
            &HashSet::from([authored[0].block_fingerprint]),
        );

        assert!(
            matches!(retried, Err(MintError::Duplicate)),
            "a match against the authored deck stays refused"
        );
    }

    #[test]
    fn record_minted_card_keeps_the_given_id_and_dedups_against_the_deck_and_the_sidecar() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "geo.md");
        let authored = crate::parser::parse_str(
            "geo.md",
            "## capital of france?\nParis\n<!-- id: card-france -->\n",
        )
        .unwrap();
        let deck_fingerprints = HashSet::from([authored[0].block_fingerprint]);
        let block = "## capital of spain?\nMadrid\n<!-- topic: topic-t -->\n<!-- id: card-6v3c7x4k1m8q3z5t0b2n4d8f9w -->\n";

        let id =
            record_minted_card(&mut store, &deck, "geo.md", block, &deck_fingerprints).unwrap();
        assert_eq!(
            "card-6v3c7x4k1m8q3z5t0b2n4d8f9w", id,
            "the block's id is kept"
        );
        assert_eq!(
            vec![Some(id.clone())],
            sidecar_ids(&deck, "geo.md")
                .into_iter()
                .map(Some)
                .collect::<Vec<_>>(),
            "the sidecar holds the card under that id"
        );
        assert!(store.get(&id).is_some(), "the store holds its entry");

        let again =
            record_minted_card(&mut store, &deck, "geo.md", block, &deck_fingerprints).unwrap_err();
        assert!(
            matches!(again, MintError::Duplicate),
            "sidecar dedup: {again:?}"
        );
        let deck_dup = record_minted_card(
            &mut store,
            &deck,
            "geo.md",
            "## capital of france?\nParis\n<!-- topic: topic-t -->\n<!-- id: card-6v3c7x4k1m8q3z5t0b2n4d8f9x -->\n",
            &deck_fingerprints,
        )
        .unwrap_err();
        assert!(
            matches!(deck_dup, MintError::Duplicate),
            "deck dedup: {deck_dup:?}"
        );
        let unstamped = record_minted_card(
            &mut store,
            &deck,
            "geo.md",
            "## capital of italy?\nRome\n",
            &deck_fingerprints,
        )
        .unwrap_err();
        assert!(
            matches!(unstamped, MintError::Malformed(_)),
            "no id: {unstamped:?}"
        );
        assert_eq!(
            1,
            sidecar_ids(&deck, "geo.md").len(),
            "nothing else reached the sidecar"
        );
    }

    /// Spec law 2, the block-splitter's row: a deck holding every
    /// structural heading splits into one block per CARD, with topics and
    /// their prose owned by no card. A splitter that only knows `## ` would
    /// swallow a sub-card into its parent and reset the wrong progress.
    #[test]
    fn split_card_blocks_cuts_at_every_card_depth() {
        let text = "# One\nprose\n## a\n1\n### b\n2\n#### c\n3\n# Two\n## d\n4\n";
        let blocks = split_card_blocks(text);
        let fronts: Vec<&str> = blocks
            .iter()
            .map(|b| b.lines().next().unwrap_or_default())
            .collect();
        assert_eq!(vec!["## a", "### b", "#### c", "## d"], fronts);
        assert!(
            !blocks.iter().any(|b| b.contains("prose")),
            "a topic's prose belongs to no card: {blocks:?}"
        );
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
    fn split_card_blocks_keeps_a_front_inside_a_nested_fence_in_its_block() {
        let blocks = split_card_blocks("## q\n````\n```\n## nope\n```\n````\n");
        assert_eq!(1, blocks.len(), "{blocks:?}");
    }

    #[test]
    fn split_card_blocks_is_one_block_for_a_span_and_drops_preamble() {
        let text = "---\nlink: x\n---\n\n## Complete the quote\nTo be or not to be\n<!-- blank: span hidden=\"be\" -->\n<!-- blank: span hidden=\"be\" occurrence=2 -->\n";
        let blocks = split_card_blocks(text);
        assert_eq!(
            1,
            blocks.len(),
            "the directives stay in the block: {blocks:?}"
        );
        assert!(blocks[0].starts_with("## Complete the quote"));
        assert!(blocks[0].contains("occurrence=2"));
    }

    fn sidecar_cards(deck: &Path, deck_id: &str) -> Vec<Card> {
        let text = std::fs::read_to_string(crate::personal::sidecar_path(deck)).unwrap_or_default();
        crate::parser::parse_sidecar(deck_id, &text)
            .map(|parsed| parsed.cards.into_iter().map(|parsed| parsed.card).collect())
            .unwrap_or_default()
    }

    fn sidecar_ids(deck: &Path, deck_id: &str) -> Vec<String> {
        sidecar_cards(deck, deck_id)
            .iter()
            .filter_map(|card| card.id())
            .collect()
    }

    fn store_remediation(
        store: &mut Store,
        deck: &Path,
        subject: &str,
        cards_text: &str,
        retire_after_days: Option<u32>,
    ) -> AnyResult<usize> {
        store_remediation_cards(
            store,
            Some(deck),
            subject,
            "topic-t",
            &std::collections::HashSet::new(),
            cards_text,
            retire_after_days,
        )
        .into_result()
    }

    #[test]
    fn failing_the_same_exam_twice_yields_zero_duplicate_gap_cards() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "d.md");
        let text = "## Why does X happen?\nbecause of Y\n";

        let first = store_remediation(&mut store, &deck, "d.md", text, None).unwrap();
        assert_eq!(1, first, "the first failure creates the gap card");
        let second = store_remediation(&mut store, &deck, "d.md", text, None).unwrap();
        assert_eq!(
            0, second,
            "the same gap again is a content dupe, not a new card"
        );
        assert_eq!(1, sidecar_cards(&deck, "d.md").len());
    }

    #[test]
    fn a_rerun_over_a_sidecar_card_this_store_never_saw_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let deck = empty_deck(&dir, "d.md");
        let text = "## Why does X happen?\nbecause of Y\n";
        let mut other_device = Store::open(dir.path().join("other.json")).unwrap();
        store_remediation(&mut other_device, &deck, "d.md", text, None).unwrap();
        let progress = dir.path().join("p.json");
        let mut store = Store::open(&progress).unwrap();

        let created = store_remediation(&mut store, &deck, "d.md", text, None).unwrap();

        assert_eq!(0, created, "the sidecar card already exists");
        assert!(
            !progress.exists(),
            "a rerun that creates nothing saves nothing"
        );
        assert_eq!(1, sidecar_cards(&deck, "d.md").len(), "no second block");
    }

    #[test]
    fn a_remediation_rerun_saves_progress_only_when_it_creates_or_revives_a_card() {
        let dir = tempfile::tempdir().unwrap();
        let progress = dir.path().join("p.json");
        let mut store = Store::open(&progress).unwrap();
        let deck = empty_deck(&dir, "d.md");
        let text = "## Why does X happen?\nbecause of Y\n";
        let cap = Some(30u32);
        let revision = || {
            let file: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&progress).unwrap()).unwrap();
            file["revision"].as_u64().unwrap()
        };

        assert_eq!(
            1,
            store_remediation(&mut store, &deck, "d.md", text, cap).unwrap()
        );
        let created = revision();

        assert_eq!(
            0,
            store_remediation(&mut store, &deck, "d.md", text, cap).unwrap()
        );
        assert_eq!(
            created,
            revision(),
            "step 2: a rerun that creates and revives nothing does not save progress"
        );

        let ids = sidecar_ids(&deck, "d.md");
        store.get_or_insert(&ids[0]).recall = Some(FsrsState {
            state: 2,
            scheduled_days: 90,
            ..Default::default()
        });
        assert_eq!(
            1,
            store_remediation(&mut store, &deck, "d.md", text, cap).unwrap()
        );
        assert_eq!(
            created + 1,
            revision(),
            "step 3: a rerun that revives a retired card saves progress once"
        );
    }

    #[test]
    fn remediation_keeps_distinct_answers_in_matching_span_blank_contexts() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "d.md");
        let lunate = "## Which bone sits in the center?\nThe lunate sits in the center\n<!-- blank: span hidden=\"lunate\" b:a1b2c3 -->\n";
        let hamate = "## Which bone sits in the center?\nThe hamate sits in the center\n<!-- blank: span hidden=\"hamate\" b:a1b2c3 -->\n";

        let first = store_remediation(&mut store, &deck, "d.md", lunate, None).unwrap();
        assert_eq!(1, first, "the first missed fact becomes a region card");
        let second = store_remediation(&mut store, &deck, "d.md", hamate, None).unwrap();
        assert_eq!(
            1, second,
            "a different answer in the same sentence shape is a distinct gap"
        );
        assert_eq!(2, sidecar_cards(&deck, "d.md").len());
    }

    #[test]
    fn an_unstamped_span_remediation_is_stamped_and_stored() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "d.md");
        let text = "## Recall how a String is laid out in memory.\nA String stores a pointer, length and capacity on the stack.\n<!-- blank: span hidden=\"pointer\" -->\n<!-- blank: span hidden=\"length\" -->\n";

        let created = store_remediation(&mut store, &deck, "d.md", text, None).unwrap();
        assert_eq!(2, created, "each generated span is stored as its own card");
        let ids = sidecar_ids(&deck, "d.md");
        assert_eq!(
            2,
            ids.len(),
            "the stored text reparses to two stamped span cards"
        );
        for id in &ids {
            assert!(
                store.get(id).is_some(),
                "the id stored in memory is the id reparsed from disk: {id}"
            );
            assert!(
                store.progress(id).is_none(),
                "an exam-written gap card is new: the drill introduces it: {id}"
            );
        }

        let rerun = store_remediation(&mut store, &deck, "d.md", text, None).unwrap();
        assert_eq!(
            0, rerun,
            "fresh stamps do not defeat dedup: the block key masks the spans"
        );
        assert_eq!(2, sidecar_ids(&deck, "d.md").len(), "no duplicate append");
    }

    #[test]
    fn remediation_does_not_duplicate_an_authored_choice_cards_fact() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let authored = crate::parser::parse_str(
            "d.md",
            "## Capital of France?\n- [x] Paris\n- [ ] Lyon\n<!-- choices: single -->\n<!-- id: card-france -->\n",
        )
        .unwrap();
        let deck_fingerprints: std::collections::HashSet<u64> =
            authored.iter().map(|card| card.block_fingerprint).collect();
        let deck = empty_deck(&dir, "d.md");

        let created = store_remediation_cards(
            &mut store,
            Some(&deck),
            "d.md",
            "topic-t",
            &deck_fingerprints,
            "## Capital of France?\nParis\n",
            None,
        )
        .into_result()
        .unwrap();

        assert_eq!(
            0, created,
            "the same canonical question and answer already exist in the authored deck"
        );
        assert!(
            !crate::personal::sidecar_path(&deck).exists(),
            "a duplicate must not create a personal file"
        );
    }

    #[test]
    fn distinct_answer_cloze_holes_stay_distinct() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "d.md");
        let text = "## Complete the quote\nTo be or not to be\n<!-- blank: span hidden=\"be\" b:a1b2c3 -->\n<!-- blank: span hidden=\"be\" occurrence=2 b:d4e5f6 -->\n";

        let n = store_remediation(&mut store, &deck, "d.md", text, None).unwrap();
        assert_eq!(2, n, "both span cards should be created, not deduped");
        let holes = sidecar_cards(&deck, "d.md");
        assert_eq!(2, holes.len());
        assert_ne!(
            holes[0].id(),
            holes[1].id(),
            "distinct ids for the two holes"
        );
        assert_eq!(
            holes[0].block_fingerprint, holes[1].block_fingerprint,
            "one block in the sidecar, two holes out of it"
        );
        assert_ne!(
            holes[0].content_fingerprint, holes[1].content_fingerprint,
            "each hole is its own effective question"
        );
    }

    #[test]
    fn a_retired_multi_hole_block_revives_every_hole() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "d.md");
        let text = "## Complete the quote\nTo be or not to bee\n<!-- blank: span hidden=\"be\" b:a1b2c3 -->\n<!-- blank: span hidden=\"bee\" b:d4e5f6 -->\n";
        let cap = Some(30u32);

        let created = store_remediation(&mut store, &deck, "d.md", text, cap).unwrap();
        assert_eq!(2, created, "both holes created on the first failure");
        let ids = sidecar_ids(&deck, "d.md");
        assert_eq!(2, ids.len());

        for id in &ids {
            store.get_or_insert(id).recall = Some(FsrsState {
                state: 2,
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

        let revived = store_remediation(&mut store, &deck, "d.md", text, cap).unwrap();
        assert_eq!(2, revived, "every retired hole revives, not just hole 0");
        for id in &ids {
            assert!(
                !crate::session::is_retired_id(id, &store, cap),
                "revived, no longer retired"
            );
            assert_eq!(
                &CardState::new(),
                store.get(id).unwrap(),
                "the hole's schedule was reset"
            );
        }
        assert_eq!(2, sidecar_cards(&deck, "d.md").len());
    }

    #[test]
    fn a_plain_card_matching_a_holes_hidden_text_does_not_suppress_remediation() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let plain =
            crate::parser::parse_str("d.md", "## Complete the quote\nbe\n<!-- id: card-p1 -->\n")
                .unwrap();
        let deck_fingerprints: std::collections::HashSet<u64> =
            plain.iter().map(|c| c.block_fingerprint).collect();

        let deck = empty_deck(&dir, "d.md");
        let cloze = "## Complete the quote\nTo be or not to bee\n<!-- blank: span hidden=\"be\" b:a1b2c3 -->\n<!-- blank: span hidden=\"bee\" b:d4e5f6 -->\n";
        let created = store_remediation_cards(
            &mut store,
            Some(&deck),
            "d.md",
            "topic-t",
            &deck_fingerprints,
            cloze,
            None,
        )
        .into_result()
        .unwrap();
        assert_eq!(
            2, created,
            "the plain card must not suppress the cloze block"
        );
        assert_eq!(2, sidecar_cards(&deck, "d.md").len());
    }

    #[test]
    fn every_sidecar_card_is_scheduled_under_the_id_its_text_reparses_to() {
        for text in [
            "## Why does X?\npoint one\n",
            "## Complete the quote\nTo be or not to bee\n<!-- blank: span hidden=\"be\" b:a1b2c3 -->\n<!-- blank: span hidden=\"bee\" b:d4e5f6 -->\n",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let deck = empty_deck(&dir, "d.md");
            std::fs::write(&deck, "## existing\nanswer\n<!-- id: card-ex1 -->\n").unwrap();
            let mut store = Store::open(dir.path().join("p.json")).unwrap();

            let created = store_remediation(&mut store, &deck, "d.md", text, None).unwrap();
            let ids = sidecar_ids(&deck, "d.md");
            assert_eq!(created, ids.len(), "{text}");

            for id in &ids {
                assert!(
                    store.get(id).is_some(),
                    "the mint filed a schedule under {id}, the id the sidecar re-parses to"
                );
            }
        }
    }

    #[test]
    fn remediation_card_reveal_is_carried() {
        use crate::depth::Reveal;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let deck = empty_deck(&dir, "d.md");
        let text =
            "## Why does X?\npoint one\n<!-- reveal: line -->\n\n## fact card\nplain answer\n";

        store_remediation(&mut store, &deck, "d.md", text, None).unwrap();
        let synthesized = sidecar_cards(&deck, "d.md");
        let lined = synthesized
            .iter()
            .find(|c| c.front == "Why does X?")
            .unwrap();
        let plain = synthesized.iter().find(|c| c.front == "fact card").unwrap();
        assert_eq!(Some(Reveal::Line), lined.reveal);
        assert_eq!(None, plain.reveal);
    }

    #[test]
    fn confidence_and_its_deck_toggle_are_written_only_when_present_and_survive_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.json");
        let mut store = Store::open(&path).unwrap();
        let state = store.get_or_insert("card-a");
        state.record_review(1000, Grade::Fail, Depth::Recall, false);
        state.history.last_mut().unwrap().confidence = Some(Confidence::Sure);
        state.record_review(2000, Grade::Pass, Depth::Recall, false);
        store.set_track_confidence("p", true);
        store.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            1,
            text.matches(r#""confidence": "sure""#).count(),
            "one marked review: {text}"
        );
        assert_eq!(
            1,
            text.matches(r#""confidence":"#).count(),
            "an unmarked review omits the key: {text}"
        );
        assert!(
            text.contains(r#""track_confidence": true"#),
            "the toggle: {text}"
        );

        let reloaded = Store::open(&path).unwrap();
        assert!(reloaded.track_confidence("p"), "toggle reloads");
        let marks: Vec<_> = reloaded
            .get("card-a")
            .unwrap()
            .history
            .iter()
            .map(|r| r.confidence)
            .collect();
        assert_eq!(vec![Some(Confidence::Sure), None], marks, "marks reload");

        let mut store = reloaded;
        store.set_track_confidence("p", false);
        store.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("track_confidence"), "off is omitted: {text}");
    }

    #[test]
    fn an_overfull_history_from_disk_trims_to_the_cap_on_the_next_review() {
        let mut state = CardState::new();
        for i in 0..(HISTORY_CAP + 5) {
            state.history.push(Review {
                ts_ms: i as u64,
                grade: Grade::Pass,
                depth: Depth::Recall,
                propagated: false,
                confidence: None,
            });
        }
        state.record_review(999_999, Grade::Pass, Depth::Recall, false);
        assert_eq!(HISTORY_CAP, state.history.len());
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
    fn tolerant_aggregate_open_still_rejects_cross_document_card_ownership_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("deck-a.json"),
            r#"{"version":1,"deck_id":"deck-a","subject":"a.md","revision":1,"cards":{"card-shared":{}}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("deck-b.json"),
            r#"{"version":1,"deck_id":"deck-b","subject":"b.md","revision":1,"cards":{"card-shared":{}}}"#,
        )
        .unwrap();

        let error = match Store::open_aggregate_tolerant(dir.path()) {
            Ok(_) => panic!("tolerance swallowed a cross-document ownership conflict"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            StoreError::DuplicateKey { kind: "card", .. }
        ));
    }

    #[test]
    fn every_partial_aggregate_save_is_valid_and_retryable() {
        let mut completed_without_a_fault = false;
        for nth in 1..=32 {
            let dir = tempfile::tempdir().unwrap();
            let paths = [
                dir.path().join("deck-a.json"),
                dir.path().join("deck-b.json"),
            ];
            for (path, deck_id) in paths.iter().zip(["deck-a", "deck-b"]) {
                std::fs::write(
                    path,
                    format!(
                        r#"{{"version":1,"deck_id":"{deck_id}","subject":"{deck_id}.md","revision":1,"cards":{{}}}}"#
                    ),
                )
                .unwrap();
            }
            let mut store = Store::open(dir.path()).unwrap();
            store.set_last_launch("deck-a", Launch::Recall);
            store.set_last_launch("deck-b", Launch::Reconstruct);

            let fault = crate::fsio::fault::fail_on_nth_operation(nth);
            let result = store.save();
            let operation = fault.triggered_operation();
            drop(fault);

            let Some(operation) = operation else {
                assert!(
                    result.is_ok(),
                    "operation {nth}: an uninjected aggregate save failed"
                );
                completed_without_a_fault = true;
                break;
            };
            assert!(
                result.is_err(),
                "operation {nth} ({operation:?}): the injected aggregate fault was swallowed"
            );
            for path in &paths {
                let text = std::fs::read_to_string(path).unwrap();
                let document: DeckStoreFile = serde_json::from_str(&text).unwrap_or_else(|error| {
                    panic!(
                        "operation {nth} ({operation:?}): {} became partial JSON: {error}",
                        path.display()
                    )
                });
                assert!(
                    matches!(document.revision, 1 | 2),
                    "operation {nth} ({operation:?}): {} has revision {}",
                    path.display(),
                    document.revision
                );
            }

            store.save().unwrap_or_else(|error| {
                panic!("operation {nth} ({operation:?}): retry failed: {error}")
            });
            let reopened = Store::open(dir.path()).unwrap();
            assert_eq!(
                Some(Launch::Recall),
                reopened.last_launch("deck-a"),
                "operation {nth} ({operation:?}): deck-a change was lost"
            );
            assert_eq!(
                Some(Launch::Reconstruct),
                reopened.last_launch("deck-b"),
                "operation {nth} ({operation:?}): deck-b change was lost"
            );
        }
        assert!(
            completed_without_a_fault,
            "the fail-on-Nth sweep never reached the successful aggregate save"
        );
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
            Some(Writer {
                device: "beta".to_string(),
                at_ms: 200,
            }),
            store.last_writer
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
            r#"{"version":1,"deck_id":"deck-b","subject":"b.md","revision":1,"cards":{"card-b1":{}},"deck":{"last_launch":"recall"}}"#,
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
        store.rebind_replaced_deck("deck-a", &replacement).unwrap();
        store.save().unwrap();

        assert!(store.get("card-b1").is_some());
        assert_eq!(Some(Launch::Recall), store.last_launch("deck-b"));

        store.set_last_launch("deck-b", Launch::Reconstruct);
        store.save().unwrap();
        let reloaded = Store::open(dir.path()).unwrap();
        assert_eq!(Some(Launch::Reconstruct), reloaded.last_launch("deck-b"));
    }

    #[test]
    fn a_stocked_store_is_not_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert("card-one").introduced_ms = Some(0);
        assert!(!store.is_empty());
    }

    #[test]
    fn every_persisted_learning_signal_engages_a_card_independently() {
        assert!(!CardState::new().engaged());
        assert!(CardState::introduced_at(0).engaged());

        for depth in [Depth::Recognize, Depth::Recall, Depth::Reconstruct] {
            let mut state = CardState::new();
            *state.schedule_slot(depth).unwrap() = Some(FsrsState::default());
            assert!(state.engaged(), "{depth:?} schedule must engage the card");
        }

        let mut reviewed = CardState::new();
        reviewed.total_reviews = 1;
        assert!(reviewed.engaged());
    }

    #[test]
    fn a_progress_document_with_exam_failed_at_ms_fails_to_parse() {
        let parsed = serde_json::from_str::<DeckProgress>(r#"{"exam_failed_at_ms":1}"#);
        assert!(parsed.is_err(), "{parsed:?}");
    }

    #[test]
    fn a_progress_document_with_a_card_exam_result_is_rejected() {
        let parsed =
            serde_json::from_str::<CardState>(r#"{"exam_result":{"at_ms":1,"pass":true}}"#);
        assert!(parsed.is_err(), "{parsed:?}");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck-old.json");
        std::fs::write(
            &path,
            r#"{"version":1,"deck_id":"deck-old","subject":"old.md","revision":1,"cards":{"card-a":{"exam_result":{"at_ms":1,"pass":true}}}}"#,
        )
        .unwrap();
        let opened = Store::open_deck(&path, "deck-old", "old.md").err();
        assert!(
            matches!(opened, Some(StoreError::Format { .. })),
            "ordinary invalid input: {opened:?}"
        );
    }

    #[test]
    fn a_topic_result_round_trips_and_a_later_result_overwrites_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        let mut store = Store::open(&path).unwrap();
        let sitting = |pass, fingerprint: &str| SittingRecord {
            topics: vec![TopicRecord {
                topic_id: "topic-a".to_string(),
                fingerprint: fingerprint.to_string(),
                pass,
            }],
            ..Default::default()
        };
        store.record_sitting(
            "deck1",
            &[],
            &sitting(true, "f1"),
            1,
            false,
            &ExamStamp::default(),
        );
        store.save().unwrap();
        let mut reloaded = Store::open(&path).unwrap();
        assert_eq!(
            Some(&ExamRecord {
                at_ms: 1,
                pass: true,
                fingerprint: "f1".to_string(),
            }),
            reloaded.topic_result("deck1", "topic-a"),
            "round trip"
        );
        reloaded.record_sitting(
            "deck1",
            &[],
            &sitting(false, "f2"),
            2,
            false,
            &ExamStamp::default(),
        );
        assert_eq!(
            Some(&ExamRecord {
                at_ms: 2,
                pass: false,
                fingerprint: "f2".to_string(),
            }),
            reloaded.topic_result("deck1", "topic-a"),
            "a later result overwrites"
        );
    }

    const TWO_TOPIC_DECK: &str = "---\nformat-version: 1\nid: deck-rec\n---\n\
        # A\n\n## a1\nx\n<!-- id: card-a1 -->\n\n## a2\nx\n<!-- id: card-a2 -->\n\n\
        # B\n\n## b1\nx\n<!-- id: card-b1 -->\n";

    fn record(deck: &Deck, examined: &[Option<bool>], counts: (u32, u32, u32)) -> SittingRecord {
        let topics = crate::exam_status::examined_topics(deck);
        SittingRecord {
            topics: topics
                .iter()
                .zip(examined)
                .filter_map(|(topic, pass)| {
                    pass.map(|pass| TopicRecord {
                        topic_id: topic.topic_id.clone(),
                        fingerprint: topic.fingerprint.clone(),
                        pass,
                    })
                })
                .collect(),
            full: examined.iter().all(Option::is_some),
            pass: counts.0,
            partial: counts.1,
            total: counts.2,
        }
    }

    fn stamp_of(deck: &Deck, store: &Store) -> ExamStamp {
        crate::exam_status::sitting_start(deck, store, 0, None).stamp
    }

    fn two_topic_deck() -> (tempfile::TempDir, Deck) {
        let (dir, decks) = crate::sources::testkit::workspace_decks();
        let deck_path = decks.join("rec.md");
        std::fs::write(&deck_path, TWO_TOPIC_DECK).unwrap();
        let deck = Deck::load(crate::sources::testkit::sourced(deck_path)).unwrap();
        (dir, deck)
    }

    #[test]
    fn recording_follows_the_four_steps() {
        type Sitting = (&'static [Option<bool>], (u32, u32, u32));
        struct Row {
            label: &'static str,
            sittings: &'static [Sitting],
            topics: [Option<bool>; 2],
            full: Option<(u64, u32, u32, u32)>,
            passed_at: Option<u64>,
            mastered_at: Option<u64>,
            stamped: bool,
            newly_passed: [bool; 2],
        }
        let rows = [
            Row {
                label: "full pass at 100%",
                sittings: &[(&[Some(true), Some(true)], (3, 0, 3))],
                topics: [Some(true), Some(true)],
                full: Some((1, 3, 0, 3)),
                passed_at: Some(1),
                mastered_at: Some(1),
                stamped: true,
                newly_passed: [true, false],
            },
            Row {
                label: "full pass below 100%",
                sittings: &[(&[Some(true), Some(true)], (2, 1, 3))],
                topics: [Some(true), Some(true)],
                full: Some((1, 2, 1, 3)),
                passed_at: Some(1),
                mastered_at: None,
                stamped: true,
                newly_passed: [true, false],
            },
            Row {
                label: "filtered pass completing every topic",
                sittings: &[
                    (&[Some(true), Some(false)], (2, 0, 3)),
                    (&[None, Some(true)], (1, 0, 1)),
                ],
                topics: [Some(true), Some(true)],
                full: Some((1, 2, 0, 3)),
                passed_at: Some(2),
                mastered_at: None,
                stamped: true,
                newly_passed: [false, true],
            },
            Row {
                label: "fail after pass",
                sittings: &[
                    (&[Some(true), Some(true)], (3, 0, 3)),
                    (&[Some(false), Some(false)], (0, 0, 3)),
                ],
                topics: [Some(false), Some(false)],
                full: Some((2, 0, 0, 3)),
                passed_at: Some(1),
                mastered_at: Some(1),
                stamped: true,
                newly_passed: [true, false],
            },
            Row {
                label: "a re-pass keeps the first instants",
                sittings: &[
                    (&[Some(true), Some(true)], (3, 0, 3)),
                    (&[Some(true), Some(true)], (3, 0, 3)),
                ],
                topics: [Some(true), Some(true)],
                full: Some((2, 3, 0, 3)),
                passed_at: Some(1),
                mastered_at: Some(1),
                stamped: true,
                newly_passed: [true, false],
            },
            Row {
                label: "a filtered sitting never writes the count",
                sittings: &[(&[None, Some(true)], (1, 0, 1))],
                topics: [None, Some(true)],
                full: None,
                passed_at: None,
                mastered_at: None,
                stamped: false,
                newly_passed: [false, false],
            },
        ];
        for row in rows {
            let (dir, deck) = two_topic_deck();
            let mut store = Store::open(dir.path().join("p.json")).unwrap();
            let stamp = stamp_of(&deck, &store);
            let mut newly = [false; 2];
            for (at, (examined, counts)) in row.sittings.iter().enumerate() {
                newly[at] = store.record_exam_result(
                    &deck,
                    &record(&deck, examined, *counts),
                    at as u64 + 1,
                    true,
                    &stamp,
                );
            }
            let label = row.label;
            let topics: Vec<Option<bool>> = crate::exam_status::examined_topics(&deck)
                .iter()
                .map(|topic| {
                    store
                        .topic_result("deck-rec", &topic.topic_id)
                        .map(|result| result.pass)
                })
                .collect();
            assert_eq!(row.topics.to_vec(), topics, "{label}: topic results");
            assert_eq!(
                row.full,
                store
                    .full_exam_result("deck-rec")
                    .map(|f| (f.at_ms, f.pass, f.partial, f.total)),
                "{label}: full_exam_result"
            );
            assert_eq!(
                row.passed_at,
                store.deck_passed_at("deck-rec"),
                "{label}: passed_at_ms"
            );
            assert_eq!(
                row.mastered_at,
                store.deck_mastered_at("deck-rec"),
                "{label}: mastered_at_ms"
            );
            assert_eq!(
                row.stamped.then_some(&stamp),
                store.exam_stamp("deck-rec"),
                "{label}: exam_stamp"
            );
            assert_eq!(row.newly_passed, newly, "{label}: newly passed per sitting");
            let standing = store.deck_passed_at("deck-rec").is_some()
                || store.deck_mastered_at("deck-rec").is_some();
            assert_eq!(
                standing,
                store.exam_stamp("deck-rec").is_some(),
                "{label}: a standing and its stamp are written together"
            );
        }
    }

    #[test]
    fn a_standing_is_never_written_without_its_stamp_and_the_document_reloads() {
        struct Row {
            label: &'static str,
            record: fn(&Deck) -> SittingRecord,
            deck_topics: bool,
            passed: bool,
            mastered: bool,
        }
        let rows = [
            Row {
                label: "a full sitting whose second sourced topic drew no question",
                record: |deck| {
                    let mut record = record(deck, &[Some(true), None], (2, 0, 2));
                    record.full = true;
                    record
                },
                deck_topics: true,
                passed: false,
                mastered: true,
            },
            Row {
                label: "a full sitting mastering the count while a recorded topic failed",
                record: |deck| record(deck, &[Some(true), Some(false)], (2, 0, 2)),
                deck_topics: true,
                passed: false,
                mastered: true,
            },
            Row {
                label: "a recorded topic id the deck lacks fails while every deck topic passes",
                record: |deck| {
                    let mut record = record(deck, &[Some(true), Some(true)], (2, 0, 3));
                    record.topics.push(TopicRecord {
                        topic_id: "topic-gone".to_string(),
                        fingerprint: "f".to_string(),
                        pass: false,
                    });
                    record
                },
                deck_topics: true,
                passed: true,
                mastered: false,
            },
            Row {
                label: "a deck with no topics",
                record: |deck| record(deck, &[Some(true), Some(true)], (3, 0, 3)),
                deck_topics: false,
                passed: true,
                mastered: true,
            },
        ];
        for row in rows {
            let label = row.label;
            let (dir, deck) = two_topic_deck();
            let path = dir.path().join("deck-rec.json");
            let mut store = Store::open(&path).unwrap();
            let stamp = stamp_of(&deck, &store);
            let topics = if row.deck_topics {
                crate::exam_status::examined_topics(&deck)
            } else {
                Vec::new()
            };
            store.record_sitting("deck-rec", &topics, &(row.record)(&deck), 1, true, &stamp);
            assert_eq!(
                row.passed,
                store.deck_passed_at("deck-rec").is_some(),
                "{label}: passed"
            );
            assert_eq!(
                row.mastered,
                store.deck_mastered_at("deck-rec").is_some(),
                "{label}: mastered"
            );
            assert_eq!(
                Some(&stamp),
                store.exam_stamp("deck-rec"),
                "{label}: the standing carries this sitting's stamp"
            );
            store.save().unwrap();
            let reopened = Store::open(&path);
            let reopened = match reopened {
                Ok(store) => store,
                Err(error) => panic!("{label}: the saved document must load again: {error}"),
            };
            assert_eq!(
                row.passed,
                reopened.deck_passed_at("deck-rec").is_some(),
                "{label}: passed survives the reload"
            );
            assert_eq!(
                row.mastered,
                reopened.deck_mastered_at("deck-rec").is_some(),
                "{label}: mastered survives the reload"
            );
        }
    }

    #[test]
    fn a_sitting_with_a_failed_topic_keeps_the_old_stamp() {
        let (dir, deck) = two_topic_deck();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let first = stamp_of(&deck, &store);
        store.record_exam_result(
            &deck,
            &record(&deck, &[Some(true), Some(true)], (3, 0, 3)),
            1,
            true,
            &first,
        );
        let later = ExamStamp {
            sourced: "later".to_string(),
            unsourced_cards: vec!["card-new".to_string()],
        };

        store.record_exam_result(
            &deck,
            &record(&deck, &[Some(true), Some(false)], (2, 0, 3)),
            2,
            true,
            &later,
        );
        assert_eq!(
            Some(&first),
            store.exam_stamp("deck-rec"),
            "a failed topic leaves the stamp as it was"
        );
        assert_eq!(
            Some(1),
            store.deck_passed_at("deck-rec"),
            "the standing is latched"
        );

        store.record_exam_result(
            &deck,
            &record(&deck, &[None, Some(true)], (1, 0, 1)),
            3,
            true,
            &later,
        );
        assert_eq!(
            Some(&later),
            store.exam_stamp("deck-rec"),
            "a passing sitting after which every topic passes replaces the stamp, passed or not"
        );
    }

    #[test]
    fn clearing_a_decks_exam_keeps_drilling_and_the_rest_of_its_progress() {
        let (dir, deck) = two_topic_deck();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let stamp = stamp_of(&deck, &store);
        store.record_exam_result(
            &deck,
            &record(&deck, &[Some(true), Some(true)], (3, 0, 3)),
            1,
            true,
            &stamp,
        );
        store.get_or_insert("card-a1").total_reviews = 4;
        store.set_last_launch("deck-rec", Launch::Recall);
        assert!(store.ever_examined(&deck), "examined before the clear");
        assert!(
            store.exam_stamp("deck-rec").is_some(),
            "setup: the sitting stamped the deck"
        );

        store.clear_deck_exam(&deck);

        assert!(!store.ever_examined(&deck), "no exam state after the clear");
        assert_eq!(None, store.deck_passed_at("deck-rec"));
        assert_eq!(None, store.deck_mastered_at("deck-rec"));
        assert_eq!(None, store.full_exam_result("deck-rec"));
        assert_eq!(None, store.exam_stamp("deck-rec"));
        assert!(
            store
                .topic_results("deck-rec")
                .is_some_and(BTreeMap::is_empty),
            "no topic result survives"
        );
        assert_eq!(
            4,
            store.get("card-a1").unwrap().total_reviews,
            "drilling kept"
        );
        assert_eq!(
            Some(Launch::Recall),
            store.last_launch("deck-rec"),
            "the deck entry's other fields kept"
        );

        let count_only = SittingRecord {
            full: true,
            total: 3,
            ..Default::default()
        };
        store.record_exam_result(&deck, &count_only, 2, true, &stamp);
        assert!(
            store.ever_examined(&deck),
            "a stored full count alone means the deck has sat its exam"
        );
    }

    #[test]
    fn clearing_a_decks_exam_clears_topic_results_and_the_stamp_in_one_write() {
        let (dir, deck) = two_topic_deck();
        let path = dir.path().join("deck-rec.json");
        let mut store = Store::open(&path).unwrap();
        let stamp = stamp_of(&deck, &store);
        store.record_exam_result(
            &deck,
            &record(&deck, &[Some(true), Some(true)], (3, 0, 3)),
            1,
            true,
            &stamp,
        );
        store.set_last_launch("deck-rec", Launch::Recall);
        store.save().unwrap();
        let before: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for key in [
            "topic_results",
            "exam_stamp",
            "passed_at_ms",
            "mastered_at_ms",
        ] {
            assert!(
                before["deck"].get(key).is_some(),
                "setup: {key} written: {before}"
            );
        }

        store.clear_deck_exam(&deck);
        store.save().unwrap();

        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for key in [
            "topic_results",
            "exam_stamp",
            "passed_at_ms",
            "mastered_at_ms",
            "full_exam_result",
        ] {
            assert!(after["deck"].get(key).is_none(), "{key} cleared: {after}");
        }
        assert_eq!("recall", after["deck"]["last_launch"], "{after}");
        Store::open(&path).expect("the cleared document loads");
    }

    #[test]
    fn a_standing_without_a_stamp_is_refused_by_every_deck_document_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck-s.json");
        let document = r#"{"version":1,"deck_id":"deck-s","subject":"s.md","revision":1,"cards":{},"deck":{"passed_at_ms":1}}"#;
        std::fs::write(&path, document).unwrap();

        let rows: Vec<(&str, Option<StoreError>)> = vec![
            (
                "Store::open_deck",
                Store::open_deck(&path, "deck-s", "s.md").err(),
            ),
            (
                "read_deck_data",
                read_deck_data(&path, "deck-s", None).err(),
            ),
            ("deck_revision", deck_revision(&path, "deck-s").err()),
            #[cfg(feature = "full")]
            (
                "ValidatedDeckDocument::parse",
                ValidatedDeckDocument::parse(document.as_bytes(), "deck-s").err(),
            ),
        ];
        for (label, error) in rows {
            assert!(
                matches!(error, Some(StoreError::StandingWithoutStamp { .. })),
                "{label}: {error:?}"
            );
        }

        let stamped = r#"{"version":1,"deck_id":"deck-s","subject":"s.md","revision":1,"cards":{},"deck":{"passed_at_ms":1,"exam_stamp":{"sourced":"x","unsourced_cards":[]}}}"#;
        std::fs::write(&path, stamped).unwrap();
        assert_eq!(
            Some(1),
            Store::open_deck(&path, "deck-s", "s.md")
                .unwrap()
                .deck_passed_at("deck-s"),
            "control: the same standing with its stamp loads"
        );
    }
}
