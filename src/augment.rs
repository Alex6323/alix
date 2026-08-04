use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{answer::Mode, card::Card, deck::Deck, depth::Reveal};

const DECK_DOCUMENT_VERSION: u32 = 1;

/// Display-only; never part of `Card::id()`, so applying it never touches progress.
/// An all-empty value still marks the card as checked, distinct from no cache entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Format {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub back: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
}

/// `Explain` (and anything else) has no reveal-axis equivalent, so it maps to `None`.
fn reveal_from_suggested(mode: Mode) -> Option<Reveal> {
    match mode {
        Mode::Flip => Some(Reveal::Flip),
        Mode::LineByLine => Some(Reveal::Line),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Augmentation {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub distractors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keypoints: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distractors_fp: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_fp: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variants_fp: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keypoints_fp: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_fp: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_fp: Option<u64>,
}

impl Augmentation {
    fn is_empty(&self) -> bool {
        self.distractors.is_empty()
            && self.group.is_none()
            && self.note.is_none()
            && self.variants.is_empty()
            && self.keypoints.is_empty()
            && self.format.is_none()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Topology {
    pub name: String,
    pub principle: String,
    pub edges: Vec<TopologyEdge>,
    pub walk: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<TopologyRegion>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub deck_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyEdge {
    pub from: String,
    pub to: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyRegion {
    pub name: String,
    pub cards: Vec<String>,
}

impl Topology {
    /// Region names in walk order, plus the index of `card`'s region. `None` if
    /// there are no regions or `card` isn't in one.
    pub fn region_path(&self, card: &str) -> Option<(Vec<&str>, usize)> {
        let current = self
            .regions
            .iter()
            .position(|r| r.cards.iter().any(|c| c == card))?;
        let names = self.regions.iter().map(|r| r.name.as_str()).collect();
        Some((names, current))
    }

    /// Scoped by owner token, not by card overlap, so a card that moved decks
    /// doesn't drag this topology along.
    pub fn belongs_to(&self, deck_tokens: &HashSet<String>) -> bool {
        !self.deck_token.is_empty() && deck_tokens.contains(&self.deck_token)
    }

    pub fn region_cards(&self, name: &str) -> Option<&[String]> {
        self.regions
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(name))
            .map(|r| r.cards.as_slice())
    }
}

#[derive(Clone, Debug, Default)]
pub struct TopologyOrder {
    rank: HashMap<String, usize>,
}

impl TopologyOrder {
    pub fn from_walk(walk: &[String]) -> Self {
        Self {
            rank: walk
                .iter()
                .enumerate()
                .map(|(i, id)| (id.clone(), i))
                .collect(),
        }
    }

    pub fn rank_of(&self, card_id: &str) -> Option<usize> {
        self.rank.get(card_id).copied()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeckAugmentFile {
    version: u32,
    deck_id: String,
    revision: u64,
    cards: HashMap<String, Augmentation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    topologies: Vec<Topology>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AugmentDocumentData {
    pub cards: HashMap<String, Augmentation>,
    pub topologies: Vec<Topology>,
}

enum AugmentBacking {
    Deck {
        deck_id: String,
        revision: AtomicU64,
    },
    Aggregate {
        documents: Mutex<Vec<AugmentDocument>>,
        card_owners: HashMap<String, String>,
    },
}

#[derive(Clone)]
struct AugmentDocument {
    path: PathBuf,
    deck_id: String,
    revision: u64,
    original: AugmentDocumentData,
}

pub struct AugmentCache {
    path: PathBuf,
    cards: HashMap<String, Augmentation>,
    topologies: Vec<Topology>,
    backing: AugmentBacking,
}

/// Loading never errors; a bad cache is silently treated as empty.
#[derive(Debug, Error)]
pub enum AugmentError {
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
    #[error("{path}: unsupported augmentation document version {version}")]
    Version { path: PathBuf, version: u32 },
    #[error("{path}: augmentation document belongs to deck `{actual}`, expected `{expected}`")]
    DeckOwner {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("{path}: stale augmentation revision {loaded}; disk is at {disk}")]
    StaleRevision {
        path: PathBuf,
        loaded: u64,
        disk: u64,
    },
    #[error("duplicate augmentation key `{key}` across per-deck documents")]
    DuplicateKey { key: String },
    #[error("cannot save aggregate augmentation: card key `{key}` has no owning deck")]
    UnownedKey { key: String },
    #[error("cannot save aggregate augmentation: topology `{name}` names unknown deck `{deck_id}`")]
    UnownedTopology { name: String, deck_id: String },
    #[error("cannot open aggregate augmentation for uninitialized deck `{subject}`")]
    MissingDeckId { subject: String },
}

fn deck_revision(path: &Path, expected_deck_id: &str) -> Result<u64, AugmentError> {
    if !path.exists() {
        return Ok(0);
    }
    let text = std::fs::read_to_string(path).map_err(|source| AugmentError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file: DeckAugmentFile =
        serde_json::from_str(&text).map_err(|source| AugmentError::Format {
            path: path.to_path_buf(),
            source,
        })?;
    if file.version != DECK_DOCUMENT_VERSION {
        return Err(AugmentError::Version {
            path: path.to_path_buf(),
            version: file.version,
        });
    }
    if file.deck_id != expected_deck_id {
        return Err(AugmentError::DeckOwner {
            path: path.to_path_buf(),
            expected: expected_deck_id.to_string(),
            actual: file.deck_id,
        });
    }
    Ok(file.revision)
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), AugmentError> {
    let io_err = |source| AugmentError::Io {
        path: path.to_path_buf(),
        source,
    };
    let json = serde_json::to_string_pretty(value).map_err(|source| AugmentError::Format {
        path: path.to_path_buf(),
        source,
    })?;
    let tmp = path.with_extension("json.tmp");
    crate::fsio::replace_file(&tmp, path, json.as_bytes()).map_err(io_err)
}

pub(crate) fn write_deck_data(
    path: &Path,
    deck_id: &str,
    revision: u64,
    data: &AugmentDocumentData,
) -> Result<(), AugmentError> {
    if let Some(dir) = path.parent() {
        crate::fsio::create_dir_all(dir).map_err(|source| AugmentError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    write_json_atomic(
        path,
        &DeckAugmentFile {
            version: DECK_DOCUMENT_VERSION,
            deck_id: deck_id.to_string(),
            revision,
            cards: data.cards.clone(),
            topologies: data.topologies.clone(),
        },
    )
}

pub(crate) fn read_deck_data(
    path: &Path,
    expected_deck_id: &str,
) -> Result<(u64, AugmentDocumentData), AugmentError> {
    let text = std::fs::read_to_string(path).map_err(|source| AugmentError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file: DeckAugmentFile =
        serde_json::from_str(&text).map_err(|source| AugmentError::Format {
            path: path.to_path_buf(),
            source,
        })?;
    if file.version != DECK_DOCUMENT_VERSION {
        return Err(AugmentError::Version {
            path: path.to_path_buf(),
            version: file.version,
        });
    }
    if file.deck_id != expected_deck_id {
        return Err(AugmentError::DeckOwner {
            path: path.to_path_buf(),
            expected: expected_deck_id.to_string(),
            actual: file.deck_id,
        });
    }
    Ok((
        file.revision,
        AugmentDocumentData {
            cards: file.cards,
            topologies: file.topologies,
        },
    ))
}

impl AugmentCache {
    pub(crate) fn empty(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            cards: HashMap::new(),
            topologies: Vec::new(),
            backing: AugmentBacking::Aggregate {
                documents: Mutex::new(Vec::new()),
                card_owners: HashMap::new(),
            },
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
            && let Some(deck_id) = crate::state::deck_id_from_document(&path)
        {
            return Self::open_deck(&path, deck_id).unwrap_or_else(|_| Self::empty(path));
        }
        Self::open_aggregate(path.clone(), HashMap::new(), HashSet::new())
            .unwrap_or_else(|_| Self::empty(path))
    }

    pub fn open_deck(
        path: impl AsRef<Path>,
        deck_id: impl Into<String>,
    ) -> Result<Self, AugmentError> {
        let path = path.as_ref().to_path_buf();
        let deck_id = deck_id.into();
        if !path.exists() {
            return Ok(Self {
                path,
                cards: HashMap::new(),
                topologies: Vec::new(),
                backing: AugmentBacking::Deck {
                    deck_id,
                    revision: AtomicU64::new(0),
                },
            });
        }
        let text = std::fs::read_to_string(&path).map_err(|source| AugmentError::Io {
            path: path.clone(),
            source,
        })?;
        let file: DeckAugmentFile =
            serde_json::from_str(&text).map_err(|source| AugmentError::Format {
                path: path.clone(),
                source,
            })?;
        if file.version != DECK_DOCUMENT_VERSION {
            return Err(AugmentError::Version {
                path,
                version: file.version,
            });
        }
        if file.deck_id != deck_id {
            return Err(AugmentError::DeckOwner {
                path,
                expected: deck_id,
                actual: file.deck_id,
            });
        }
        Ok(Self {
            path,
            cards: file.cards,
            topologies: file.topologies,
            backing: AugmentBacking::Deck {
                deck_id,
                revision: AtomicU64::new(file.revision),
            },
        })
    }

    fn open_aggregate(
        path: PathBuf,
        mut card_owners: HashMap<String, String>,
        expected_decks: HashSet<String>,
    ) -> Result<Self, AugmentError> {
        let mut document_paths: Vec<PathBuf> = if path.is_dir() {
            std::fs::read_dir(&path)
                .map_err(|source| AugmentError::Io {
                    path: path.clone(),
                    source,
                })?
                .map(|entry| {
                    entry
                        .map(|entry| entry.path())
                        .map_err(|source| AugmentError::Io {
                            path: path.clone(),
                            source,
                        })
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        document_paths.retain(|document_path| {
            document_path.is_file()
                && document_path.extension().is_some_and(|ext| ext == "json")
                && document_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_none_or(|name| !crate::workspace::is_conflict_name(name))
        });
        document_paths.sort();
        let mut cards = HashMap::new();
        let mut topologies = Vec::new();
        let mut documents = Vec::new();
        let mut loaded_decks = HashSet::new();
        for document_path in document_paths {
            let Some(deck_id) =
                crate::state::deck_id_from_document(&document_path).map(str::to_string)
            else {
                continue;
            };
            let (revision, data) = read_deck_data(&document_path, &deck_id)?;
            for (key, value) in &data.cards {
                if cards.insert(key.clone(), value.clone()).is_some()
                    || card_owners
                        .insert(key.clone(), deck_id.clone())
                        .is_some_and(|owner| owner != deck_id)
                {
                    return Err(AugmentError::DuplicateKey { key: key.clone() });
                }
            }
            for topology in &data.topologies {
                if topology.deck_token != deck_id {
                    return Err(AugmentError::DeckOwner {
                        path: document_path.clone(),
                        expected: deck_id,
                        actual: topology.deck_token.clone(),
                    });
                }
            }
            topologies.extend(data.topologies.clone());
            loaded_decks.insert(deck_id.clone());
            documents.push(AugmentDocument {
                path: document_path,
                deck_id,
                revision,
                original: data,
            });
        }
        for deck_id in expected_decks.difference(&loaded_decks) {
            documents.push(AugmentDocument {
                path: path.join(format!("{deck_id}.json")),
                deck_id: deck_id.clone(),
                revision: 0,
                original: AugmentDocumentData::default(),
            });
        }
        documents.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self {
            path,
            cards,
            topologies,
            backing: AugmentBacking::Aggregate {
                documents: Mutex::new(documents),
                card_owners,
            },
        })
    }

    pub fn open_for_decks(workspace_root: &Path, decks: &[Deck]) -> Result<Self, AugmentError> {
        let path = crate::workspace::WorkspaceFiles::new(workspace_root).augment();
        let mut owners = HashMap::new();
        let mut deck_ids = HashSet::new();
        for deck in decks {
            let deck_id =
                deck.deck_token
                    .as_deref()
                    .ok_or_else(|| AugmentError::MissingDeckId {
                        subject: deck.subject.clone(),
                    })?;
            deck_ids.insert(deck_id.to_string());
            for card_id in deck.cards.iter().filter_map(Card::id) {
                if owners
                    .insert(card_id.clone(), deck_id.to_string())
                    .is_some_and(|owner| owner != deck_id)
                {
                    return Err(AugmentError::DuplicateKey { key: card_id });
                }
            }
        }
        Self::open_aggregate(path, owners, deck_ids)
    }

    pub fn open_for_workspace(workspace_root: &Path) -> Result<Self, AugmentError> {
        let path = crate::workspace::WorkspaceFiles::new(workspace_root).augment();
        Self::open_aggregate(path, HashMap::new(), HashSet::new())
    }

    pub fn open_for_deck(deck: &Deck) -> Result<Self, AugmentError> {
        let deck_id = deck
            .deck_token
            .as_deref()
            .ok_or_else(|| AugmentError::MissingDeckId {
                subject: deck.subject.clone(),
            })?;
        let path = crate::workspace::WorkspaceFiles::for_deck(&deck.path).augment_for(deck_id);
        Self::open_deck(path, deck_id)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn save(&self) -> Result<(), AugmentError> {
        match &self.backing {
            AugmentBacking::Deck { deck_id, revision } => self.save_deck(deck_id, revision),
            AugmentBacking::Aggregate {
                documents,
                card_owners,
            } => self.save_aggregate(documents, card_owners),
        }
    }

    fn save_deck(&self, deck_id: &str, revision: &AtomicU64) -> Result<(), AugmentError> {
        let io_err = |source| AugmentError::Io {
            path: self.path.clone(),
            source,
        };
        if let Some(dir) = self.path.parent() {
            crate::fsio::create_dir_all(dir).map_err(io_err)?;
        }
        let loaded = revision.load(Ordering::Relaxed);
        let disk = deck_revision(&self.path, deck_id)?;
        if loaded != disk {
            return Err(AugmentError::StaleRevision {
                path: self.path.clone(),
                loaded,
                disk,
            });
        }
        let next = loaded.saturating_add(1);
        let file = DeckAugmentFile {
            version: DECK_DOCUMENT_VERSION,
            deck_id: deck_id.to_string(),
            revision: next,
            cards: self.cards.clone(),
            topologies: self.topologies.clone(),
        };
        write_json_atomic(&self.path, &file)?;
        revision.store(next, Ordering::Relaxed);
        Ok(())
    }

    fn save_aggregate(
        &self,
        documents: &Mutex<Vec<AugmentDocument>>,
        card_owners: &HashMap<String, String>,
    ) -> Result<(), AugmentError> {
        if let Some(key) = self
            .cards
            .keys()
            .find(|key| !card_owners.contains_key(*key))
        {
            return Err(AugmentError::UnownedKey { key: key.clone() });
        }
        {
            let documents = documents
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let known_decks: HashSet<&str> = documents
                .iter()
                .map(|document| document.deck_id.as_str())
                .collect();
            if let Some(topology) = self
                .topologies
                .iter()
                .find(|topology| !known_decks.contains(topology.deck_token.as_str()))
            {
                return Err(AugmentError::UnownedTopology {
                    name: topology.name.clone(),
                    deck_id: topology.deck_token.clone(),
                });
            }
        }

        let mut documents = documents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut changed = Vec::new();
        for (index, document) in documents.iter().enumerate() {
            let data = AugmentDocumentData {
                cards: self
                    .cards
                    .iter()
                    .filter(|(key, _)| {
                        card_owners
                            .get(*key)
                            .is_some_and(|owner| owner == &document.deck_id)
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
                topologies: self
                    .topologies
                    .iter()
                    .filter(|topology| topology.deck_token == document.deck_id)
                    .cloned()
                    .collect(),
            };
            if data != document.original {
                let disk = deck_revision(&document.path, &document.deck_id)?;
                if disk != document.revision {
                    return Err(AugmentError::StaleRevision {
                        path: document.path.clone(),
                        loaded: document.revision,
                        disk,
                    });
                }
                changed.push((index, data));
            }
        }
        for (index, data) in changed {
            let document = &mut documents[index];
            let next = document.revision.saturating_add(1);
            write_deck_data(&document.path, &document.deck_id, next, &data)?;
            document.revision = next;
            document.original = data;
        }
        Ok(())
    }

    /// `None` when absent or empty, so the caller can fall back to offline
    /// sampling with one check.
    pub fn distractors(&self, card_id: &str, fingerprint: u64) -> Option<&[String]> {
        self.cards.get(card_id).and_then(|aug| {
            (aug.distractors_fp == Some(fingerprint) && !aug.distractors.is_empty())
                .then_some(aug.distractors.as_slice())
        })
    }

    pub fn contains(&self, card_id: &str) -> bool {
        self.cards.contains_key(card_id)
    }

    pub fn group(&self, card_id: &str, fingerprint: u64) -> Option<&str> {
        let aug = self.cards.get(card_id)?;
        (aug.group_fp == Some(fingerprint))
            .then_some(aug.group.as_deref())
            .flatten()
    }

    pub fn set_group(&mut self, card_id: &str, group: String, fingerprint: u64) {
        let aug = self.cards.entry(card_id.to_string()).or_default();
        aug.group = Some(group);
        aug.group_fp = Some(fingerprint);
    }

    pub fn set_distractors(&mut self, card_id: &str, distractors: Vec<String>, fingerprint: u64) {
        let aug = self.cards.entry(card_id.to_string()).or_default();
        aug.distractors = distractors;
        aug.distractors_fp = Some(fingerprint);
    }

    pub fn note(&self, card_id: &str, fingerprint: u64) -> Option<&str> {
        self.cards
            .get(card_id)
            .filter(|aug| aug.note_fp == Some(fingerprint))
            .and_then(|aug| aug.note.as_deref())
    }

    pub fn set_note(&mut self, card_id: &str, note: String, fingerprint: u64) {
        let aug = self.cards.entry(card_id.to_string()).or_default();
        aug.note = Some(note);
        aug.note_fp = Some(fingerprint);
    }

    pub fn format(&self, card_id: &str, fingerprint: u64) -> Option<&Format> {
        self.cards
            .get(card_id)
            .filter(|aug| aug.format_fp == Some(fingerprint))
            .and_then(|aug| aug.format.as_ref())
    }

    pub fn set_format(&mut self, card_id: &str, format: Format, fingerprint: u64) {
        let aug = self.cards.entry(card_id.to_string()).or_default();
        aug.format = Some(format);
        aug.format_fp = Some(fingerprint);
    }

    pub fn variants(&self, card_id: &str, fingerprint: u64) -> Option<&[String]> {
        self.cards.get(card_id).and_then(|aug| {
            (aug.variants_fp == Some(fingerprint) && !aug.variants.is_empty())
                .then_some(aug.variants.as_slice())
        })
    }

    /// The pool is `original` at index 0 plus the cached variants; picks by
    /// `seed % pool_len`. `None` when no variants are cached.
    pub fn pick_front(
        &self,
        card_id: &str,
        original: &str,
        seed: u64,
        fingerprint: u64,
    ) -> Option<String> {
        let variants = self.variants(card_id, fingerprint)?;
        let pool_len = variants.len() + 1; // + the original at index 0
        let idx = (seed % pool_len as u64) as usize;
        Some(if idx == 0 {
            original.to_string()
        } else {
            variants[idx - 1].clone()
        })
    }

    pub fn set_variants(&mut self, card_id: &str, variants: Vec<String>, fingerprint: u64) {
        let aug = self.cards.entry(card_id.to_string()).or_default();
        aug.variants = variants;
        aug.variants_fp = Some(fingerprint);
    }

    pub fn keypoints(&self, card_id: &str, fingerprint: u64) -> Option<&[String]> {
        self.cards.get(card_id).and_then(|aug| {
            (aug.keypoints_fp == Some(fingerprint) && !aug.keypoints.is_empty())
                .then_some(aug.keypoints.as_slice())
        })
    }

    pub fn set_keypoints(&mut self, card_id: &str, keypoints: Vec<String>, fingerprint: u64) {
        let aug = self.cards.entry(card_id.to_string()).or_default();
        aug.keypoints = keypoints;
        aug.keypoints_fp = Some(fingerprint);
    }

    pub fn topologies(&self) -> &[Topology] {
        &self.topologies
    }

    pub fn topologies_for(&self, deck_tokens: &HashSet<String>) -> Vec<&Topology> {
        self.topologies
            .iter()
            .filter(|t| t.belongs_to(deck_tokens))
            .collect()
    }

    pub fn has_topology_for(&self, deck_tokens: &HashSet<String>) -> bool {
        self.topologies.iter().any(|t| t.belongs_to(deck_tokens))
    }

    pub fn topology(&self, name: &str) -> Option<&Topology> {
        self.topologies.iter().find(|t| t.name == name)
    }

    /// Replaces an existing topology with the same name **and** owner deck token
    /// (so a like-named topology from another deck sharing this cache survives); otherwise appends.
    pub fn add_topology(&mut self, topology: Topology) {
        match self
            .topologies
            .iter_mut()
            .find(|t| t.name == topology.name && t.deck_token == topology.deck_token)
        {
            Some(existing) => *existing = topology,
            None => self.topologies.push(topology),
        }
    }

    /// Realigns a cloze card's hole-keyed cache entries after its holes shift:
    /// matched holes MOVE to their new index, orphaned holes' entries drop, fresh holes start
    /// empty.
    pub fn remap_holes(&mut self, token: &str, outcome: &crate::store::CascadeOutcome) -> bool {
        // An identity remap with no orphans really is a no-op: nothing moves.
        let identity = outcome.remap.iter().all(|(from, to)| from == to);
        if identity && outcome.orphaned.is_empty() {
            return false;
        }
        let moves: HashMap<u32, u32> = outcome.remap.iter().copied().collect();

        // Pulled into a temp Vec first (not rewritten in place) so a hole moving
        // into another's old slot can't clobber it before that entry is read.
        let stored: Vec<u32> = moves
            .keys()
            .copied()
            .chain(outcome.orphaned.iter().copied())
            .collect();
        let mut pulled: Vec<(u32, Augmentation)> = Vec::new();
        for n in &stored {
            if let Some(aug) = self
                .cards
                .remove(&crate::token::card_id(token, Some(*n), false))
            {
                pulled.push((*n, aug));
            }
        }
        for (from, aug) in pulled {
            if let Some(to) = moves.get(&from) {
                self.cards
                    .insert(crate::token::card_id(token, Some(*to), false), aug);
            }
        }

        let remap_id = |id: &str| -> Option<String> {
            match crate::token::parse_prefixed_card_id(id) {
                Some((t, Some(n), false)) if t == token => moves
                    .get(&n)
                    .map(|to| crate::token::card_id(token, Some(*to), false)),
                _ => Some(id.to_string()),
            }
        };
        for topo in &mut self.topologies {
            topo.walk.retain(|id| remap_id(id).is_some());
            for slot in &mut topo.walk {
                if let Some(new) = remap_id(slot) {
                    *slot = new;
                }
            }
            topo.edges
                .retain(|e| remap_id(&e.from).is_some() && remap_id(&e.to).is_some());
            for edge in &mut topo.edges {
                if let Some(new) = remap_id(&edge.from) {
                    edge.from = new;
                }
                if let Some(new) = remap_id(&edge.to) {
                    edge.to = new;
                }
            }
            for region in &mut topo.regions {
                region.cards.retain(|id| remap_id(id).is_some());
                for slot in &mut region.cards {
                    if let Some(new) = remap_id(slot) {
                        *slot = new;
                    }
                }
            }
        }
        true
    }

    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// Coverage per target, scoped to `cards`: the cache file may be shared by
    /// other decks on the same store.
    pub fn summarize(&self, cards: &[Card], deck_tokens: &HashSet<String>) -> CoverageSummary {
        let coverage = |eligible: &[&Card], covered: &dyn Fn(&Card) -> bool| Coverage {
            covered: eligible.iter().filter(|c| covered(c)).count(),
            eligible: eligible.len(),
        };
        let all: Vec<&Card> = cards.iter().collect();
        let plain: Vec<&Card> = cards.iter().filter(|c| c.hash_lines.is_none()).collect();
        CoverageSummary {
            choices: coverage(&all, &|c| {
                !c.authored_distractors.is_empty()
                    || c.id()
                        .is_some_and(|id| self.distractors(&id, c.content_fingerprint).is_some())
            }),
            notes: coverage(&all, &|c| {
                c.id()
                    .is_some_and(|id| self.note(&id, c.content_fingerprint).is_some())
            }),
            questions: coverage(&plain, &|c| {
                c.id()
                    .is_some_and(|id| self.variants(&id, c.content_fingerprint).is_some())
            }),
            keypoints: coverage(&all, &|c| {
                c.id()
                    .is_some_and(|id| self.keypoints(&id, c.content_fingerprint).is_some())
            }),
            format: coverage(&plain, &|c| {
                c.id()
                    .is_some_and(|id| self.format(&id, c.content_fingerprint).is_some())
            }),
            topologies: self
                .topologies_for(deck_tokens)
                .iter()
                .map(|t| t.name.clone())
                .collect(),
        }
    }

    /// A card a generator legitimately skips (no usable distractor, an atomic
    /// answer) stays "missing" and is retried by later fill-the-gaps runs; that's accepted, not a
    /// bug.
    fn missing(
        &self,
        cards: &[Card],
        eligible: impl Fn(&Card) -> bool,
        covered: impl Fn(&Card) -> bool,
    ) -> Vec<WarmItem> {
        cards
            .iter()
            .filter(|c| eligible(c) && !covered(c))
            .map(WarmItem::from_card)
            .collect()
    }

    pub fn missing_choices(&self, cards: &[Card]) -> Vec<WarmItem> {
        self.missing(
            cards,
            |_| true,
            |c| {
                !c.authored_distractors.is_empty()
                    || c.id()
                        .is_some_and(|id| self.distractors(&id, c.content_fingerprint).is_some())
                    || crate::choice::can_build_grouped(c, cards, self)
            },
        )
    }

    pub fn missing_notes(&self, cards: &[Card]) -> Vec<WarmItem> {
        self.missing(
            cards,
            |_| true,
            |c| {
                c.id()
                    .is_some_and(|id| self.note(&id, c.content_fingerprint).is_some())
            },
        )
    }

    pub fn missing_questions(&self, cards: &[Card]) -> Vec<WarmItem> {
        self.missing(
            cards,
            |c| c.hash_lines.is_none(),
            |c| {
                c.id()
                    .is_some_and(|id| self.variants(&id, c.content_fingerprint).is_some())
            },
        )
    }

    pub fn missing_keypoints(&self, cards: &[Card]) -> Vec<WarmItem> {
        self.missing(
            cards,
            |_| true,
            |c| {
                c.id()
                    .is_some_and(|id| self.keypoints(&id, c.content_fingerprint).is_some())
            },
        )
    }

    pub fn missing_format(&self, cards: &[Card]) -> Vec<WarmItem> {
        self.missing(
            cards,
            |c| c.hash_lines.is_none(),
            |c| {
                c.id()
                    .is_some_and(|id| self.format(&id, c.content_fingerprint).is_some())
            },
        )
    }

    /// Scoped to `deck_ids` since the cache file may be shared by other decks.
    pub fn clear_distractors(&mut self, deck_ids: &HashSet<String>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.distractors.clear();
                aug.group = None;
                aug.group_fp = None;
            }
        }
        self.prune_empty(deck_ids);
    }

    pub fn clear_notes(&mut self, deck_ids: &HashSet<String>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.note = None;
            }
        }
        self.prune_empty(deck_ids);
    }

    pub fn clear_variants(&mut self, deck_ids: &HashSet<String>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.variants.clear();
            }
        }
        self.prune_empty(deck_ids);
    }

    pub fn clear_keypoints(&mut self, deck_ids: &HashSet<String>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.keypoints.clear();
            }
        }
        self.prune_empty(deck_ids);
    }

    pub fn clear_format(&mut self, deck_ids: &HashSet<String>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.format = None;
            }
        }
        self.prune_empty(deck_ids);
    }

    /// Never touches `card.back`, so applying a reshape never changes `card.id()`.
    pub fn apply_format(&self, card: &mut Card) {
        let Some(fmt) = card
            .id()
            .and_then(|id| self.format(&id, card.content_fingerprint))
        else {
            return;
        };
        if let Some(front) = &fmt.front {
            card.front = front.clone();
        }
        if let Some(note) = &fmt.note {
            card.note = Some(note.clone());
        }
        if !fmt.back.is_empty() {
            card.display_back = Some(fmt.back.clone());
        }
        if card.reveal.is_none() {
            card.reveal = fmt.mode.and_then(reveal_from_suggested);
        }
    }

    fn prune_empty(&mut self, deck_ids: &HashSet<String>) {
        for id in deck_ids {
            if self.cards.get(id).is_some_and(Augmentation::is_empty) {
                self.cards.remove(id);
            }
        }
    }

    pub fn remove_topology(&mut self, name: &str, deck_tokens: &HashSet<String>) -> bool {
        let before = self.topologies.len();
        self.topologies
            .retain(|t| !(t.name == name && t.belongs_to(deck_tokens)));
        self.topologies.len() != before
    }

    pub fn clear_all(&mut self, deck_ids: &HashSet<String>, deck_tokens: &HashSet<String>) {
        for id in deck_ids {
            self.cards.remove(id);
        }
        self.topologies.retain(|t| !t.belongs_to(deck_tokens));
    }

    pub fn remove_cards(&mut self, card_ids: &HashSet<String>) -> bool {
        let cards_before = self.cards.len();
        self.cards.retain(|id, _| !card_ids.contains(id));
        let mut changed = self.cards.len() != cards_before;
        for topology in &mut self.topologies {
            let walk_before = topology.walk.len();
            topology.walk.retain(|id| !card_ids.contains(id));
            let edges_before = topology.edges.len();
            topology
                .edges
                .retain(|edge| !card_ids.contains(&edge.from) && !card_ids.contains(&edge.to));
            let regions_before = topology
                .regions
                .iter()
                .map(|region| region.cards.len())
                .sum::<usize>();
            for region in &mut topology.regions {
                region.cards.retain(|id| !card_ids.contains(id));
            }
            changed |= walk_before != topology.walk.len()
                || edges_before != topology.edges.len()
                || regions_before
                    != topology
                        .regions
                        .iter()
                        .map(|region| region.cards.len())
                        .sum::<usize>();
        }
        changed
    }

    /// Token-scoped, unlike [`clear_all`](Self::clear_all)'s exact ids, so a
    /// stale entry under a wiped token goes too. Does not save.
    pub fn wipe_tokens(
        &mut self,
        card_tokens: &HashSet<String>,
        deck_tokens: &HashSet<String>,
    ) -> bool {
        let cards_before = self.cards.len();
        self.cards.retain(|id, _| {
            !crate::token::parse_prefixed_card_id(id)
                .is_some_and(|(token, _, _)| card_tokens.contains(token))
        });
        let topos_before = self.topologies.len();
        self.topologies.retain(|t| !t.belongs_to(deck_tokens));
        self.cards.len() != cards_before || self.topologies.len() != topos_before
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coverage {
    pub covered: usize,
    pub eligible: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageSummary {
    pub choices: Coverage,
    pub notes: Coverage,
    pub questions: Coverage,
    pub keypoints: Coverage,
    pub format: Coverage,
    pub topologies: Vec<String>,
}

pub fn sync_conflicts(workspace_root: &Path) -> Vec<PathBuf> {
    let mut conflicts: Vec<PathBuf> =
        std::fs::read_dir(crate::workspace::WorkspaceFiles::new(workspace_root).augment())
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
    conflicts.sort();
    conflicts
}

#[derive(Clone, Debug)]
pub struct WarmItem {
    /// Always non-empty in practice: warm items are only built over already-stamped cards.
    pub id: String,
    pub question: String,
    pub answer: String,
    pub note: Option<String>,
}

impl WarmItem {
    pub fn from_card(card: &Card) -> Self {
        Self {
            id: card.id().unwrap_or_default(),
            question: card.front.clone(),
            answer: card.back.join("\n"),
            note: card.note.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: u64 = 1;

    #[test]
    fn reveal_from_suggested_maps_only_flip_and_line() {
        assert_eq!(Some(Reveal::Flip), reveal_from_suggested(Mode::Flip));
        assert_eq!(Some(Reveal::Line), reveal_from_suggested(Mode::LineByLine));
        assert_eq!(None, reveal_from_suggested(Mode::Explain));
        assert_eq!(None, reveal_from_suggested(Mode::Typing));
    }

    #[test]
    fn open_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AugmentCache::open(dir.path().join("deck1.json"));
        assert!(cache.is_empty());
    }

    #[test]
    fn sync_conflicts_finds_per_deck_augmentation_copies() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("augment")).unwrap();
        let conflict = dir
            .path()
            .join("augment/deck2.sync-conflict-20260714-laptop.json");
        std::fs::write(&conflict, "{}").unwrap();

        assert_eq!(sync_conflicts(dir.path()), vec![conflict]);
    }

    #[test]
    fn augment_entries_move_with_their_hole() {
        use crate::store::CascadeOutcome;
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        cache.set_distractors("tok-0", vec!["wrong x".into(), "wrong y".into()], FP);
        cache.set_note("tok-0", "a note about the old hole 0".into(), FP);

        let outcome = CascadeOutcome {
            remap: vec![(0, 1)],
            orphaned: vec![],
            fresh: vec![0],
        };
        assert!(cache.remap_holes("tok", &outcome));

        assert_eq!(
            Some(["wrong x".to_string(), "wrong y".to_string()].as_slice()),
            cache.distractors("tok-1", FP)
        );
        assert_eq!(Some("a note about the old hole 0"), cache.note("tok-1", FP));
        assert!(cache.distractors("tok-0", FP).is_none());
        assert!(cache.note("tok-0", FP).is_none());
    }

    #[test]
    fn an_orphaned_holes_augmentation_is_dropped_not_inherited() {
        use crate::store::CascadeOutcome;
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        cache.set_distractors("tok-0", vec!["a".into()], FP);
        let outcome = CascadeOutcome {
            remap: vec![],
            orphaned: vec![0],
            fresh: vec![0],
        };
        assert!(cache.remap_holes("tok", &outcome));
        assert!(cache.distractors("tok-0", FP).is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");

        let mut cache = AugmentCache::open(&path);
        cache.set_distractors("c42", vec!["wrong a".into(), "wrong b".into()], FP);
        cache.save().unwrap();

        let reloaded = AugmentCache::open(&path);
        assert_eq!(1, reloaded.len());
        assert_eq!(
            Some(["wrong a".to_string(), "wrong b".to_string()].as_slice()),
            reloaded.distractors("c42", FP)
        );
    }

    #[test]
    fn deck_document_roundtrip_records_owner_and_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment/deck1.json");
        let mut cache = AugmentCache::open_deck(&path, "deck1").unwrap();
        cache.set_note("card1", "note".to_string(), FP);
        cache.save().unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\": 1"));
        assert!(text.contains("\"deck_id\": \"deck1\""));
        assert!(text.contains("\"revision\": 1"));
        let reopened = AugmentCache::open_deck(&path, "deck1").unwrap();
        assert_eq!(Some("note"), reopened.note("card1", FP));
    }

    #[test]
    fn a_deck_augmentation_document_refuses_the_wrong_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment/deck1.json");
        AugmentCache::open_deck(&path, "deck1")
            .unwrap()
            .save()
            .unwrap();

        let error = match AugmentCache::open_deck(&path, "deck2") {
            Ok(_) => panic!("wrong owner was accepted"),
            Err(error) => error,
        };
        assert!(matches!(error, AugmentError::DeckOwner { .. }));
    }

    #[test]
    fn a_stale_augmentation_save_does_not_replace_the_newer_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment/deck1.json");
        AugmentCache::open_deck(&path, "deck1")
            .unwrap()
            .save()
            .unwrap();
        let mut first = AugmentCache::open_deck(&path, "deck1").unwrap();
        let mut stale = AugmentCache::open_deck(&path, "deck1").unwrap();
        first.set_note("card1", "newer".to_string(), FP);
        first.save().unwrap();
        stale.set_note("card1", "stale".to_string(), FP);

        let error = match stale.save() {
            Ok(()) => panic!("stale revision was accepted"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            AugmentError::StaleRevision {
                loaded: 1,
                disk: 2,
                ..
            }
        ));
        let reopened = AugmentCache::open_deck(&path, "deck1").unwrap();
        assert_eq!(Some("newer"), reopened.note("card1", FP));
    }

    #[test]
    fn a_truncated_augmentation_document_is_rejected_not_panicked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment/deck1.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"version":1,"deck_id":"deck1","car"#).unwrap();

        let error = match AugmentCache::open_deck(&path, "deck1") {
            Ok(_) => panic!("a truncated document was accepted"),
            Err(error) => error,
        };
        assert!(matches!(error, AugmentError::Format { .. }));
    }

    #[test]
    fn distractors_is_none_when_absent_or_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        assert_eq!(None, cache.distractors("c1", FP));
        cache.set_distractors("c1", Vec::new(), FP);
        assert_eq!(None, cache.distractors("c1", FP));
        assert!(cache.contains("c1"));
    }

    #[test]
    fn corrupt_file_yields_empty_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        std::fs::write(&path, "this is not json").unwrap();
        let cache = AugmentCache::open(&path);
        assert!(cache.is_empty());
    }

    #[test]
    fn newer_version_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        std::fs::write(
            &path,
            r#"{"version":999,"deck_id":"deck1","revision":1,"cards":{}}"#,
        )
        .unwrap();
        let cache = AugmentCache::open(&path);
        assert!(cache.is_empty());
    }

    #[test]
    fn every_string_key_loads_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        let mut cache = AugmentCache::open(&path);
        cache.set_distractors("not-a-token", vec!["x".to_string()], FP);
        cache.set_distractors("q7", vec!["y".to_string()], FP);
        cache.save().unwrap();
        let cache = AugmentCache::open(&path);
        assert_eq!(2, cache.len());
        assert!(cache.contains("q7"));
        assert!(cache.contains("not-a-token"));
        assert_eq!(Some(&["y".to_string()][..]), cache.distractors("q7", FP));
        assert_eq!(
            Some(&["x".to_string()][..]),
            cache.distractors("not-a-token", FP)
        );
    }

    #[test]
    fn set_distractors_replaces_previous() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        cache.set_distractors("c1", vec!["old".into()], FP);
        cache.set_distractors("c1", vec!["new a".into(), "new b".into()], FP);
        assert_eq!(
            Some(["new a".to_string(), "new b".to_string()].as_slice()),
            cache.distractors("c1", FP)
        );
    }

    #[test]
    fn note_roundtrips_through_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        let mut cache = AugmentCache::open(&path);
        cache.set_note("c7", "a memorable fact".into(), FP);
        cache.save().unwrap();
        let reloaded = AugmentCache::open(&path);
        assert_eq!(Some("a memorable fact"), reloaded.note("c7", FP));
        assert_eq!(None, reloaded.note("c8", FP));
    }

    #[test]
    fn variants_roundtrip_and_pick() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        let mut cache = AugmentCache::open(&path);
        cache.set_variants("c5", vec!["one".into(), "two".into(), "three".into()], FP);
        cache.save().unwrap();
        let reloaded = AugmentCache::open(&path);
        assert_eq!(3, reloaded.variants("c5", FP).unwrap().len());
        assert_eq!(
            Some("ORIG".to_string()),
            reloaded.pick_front("c5", "ORIG", 0, FP)
        );
        assert_eq!(
            Some("one".to_string()),
            reloaded.pick_front("c5", "ORIG", 1, FP)
        );
        assert_eq!(
            Some("ORIG".to_string()),
            reloaded.pick_front("c5", "ORIG", 4, FP)
        );
        assert_eq!(None, reloaded.pick_front("c6", "ORIG", 0, FP));
    }

    #[test]
    fn keypoints_roundtrip_through_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        let mut cache = AugmentCache::open(&path);
        cache.set_keypoints("c9", vec!["claim a".into(), "claim b".into()], FP);
        cache.save().unwrap();
        let reloaded = AugmentCache::open(&path);
        assert_eq!(
            Some(["claim a".to_string(), "claim b".to_string()].as_slice()),
            reloaded.keypoints("c9", FP)
        );
        assert_eq!(None, reloaded.keypoints("c10", FP));
    }

    fn tokens(ts: &[&str]) -> HashSet<String> {
        ts.iter().map(|s| s.to_string()).collect()
    }

    fn topology(name: &str, deck_token: &str, walk: &[&str]) -> Topology {
        Topology {
            name: name.into(),
            principle: format!("principle for {name}"),
            edges: vec![TopologyEdge {
                from: walk[0].into(),
                to: walk[1].into(),
                label: "x".into(),
            }],
            walk: walk.iter().map(|s| s.to_string()).collect(),
            regions: Vec::new(),
            deck_token: deck_token.into(),
        }
    }

    fn region(name: &str, cards: &[&str]) -> TopologyRegion {
        TopologyRegion {
            name: name.into(),
            cards: cards.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn topo_regions(regions: Vec<TopologyRegion>) -> Topology {
        Topology {
            name: "n".into(),
            principle: "p".into(),
            edges: Vec::new(),
            walk: Vec::new(),
            regions,
            deck_token: "d1".into(),
        }
    }

    fn region_ids<'a>(t: &'a Topology, name: &str) -> Vec<&'a str> {
        t.region_cards(name)
            .unwrap()
            .iter()
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn topology_roundtrips_through_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        let mut cache = AugmentCache::open(&path);
        assert!(cache.topologies().is_empty());
        cache.add_topology(topology("auto", "d1", &["c1", "c2"]));
        cache.save().unwrap();

        let reloaded = AugmentCache::open(&path);
        let t = reloaded.topology("auto").unwrap();
        assert_eq!("principle for auto", t.principle);
        assert_eq!(t.walk, ["c1", "c2"]);
        assert_eq!("d1", t.deck_token);
        assert_eq!(1, t.edges.len());
    }

    #[test]
    fn add_topology_appends_new_names_and_replaces_same_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        cache.add_topology(topology("north to south", "d1", &["c1", "c2"]));
        cache.add_topology(topology("by continent", "d1", &["c3", "c4"]));
        assert_eq!(2, cache.topologies().len());

        cache.add_topology(topology("north to south", "d1", &["c1", "c2", "c7"]));
        assert_eq!(2, cache.topologies().len());
        assert_eq!(
            cache.topology("north to south").unwrap().walk,
            ["c1", "c2", "c7"]
        );
        assert_eq!(cache.topology("by continent").unwrap().walk, ["c3", "c4"]);
        assert!(cache.topology("alphabetical").is_none());
    }

    #[test]
    fn add_topology_keeps_like_named_topologies_for_different_decks() {
        let mut cache = AugmentCache::open(std::path::Path::new("unused.json"));
        cache.add_topology(topology("auto", "dA", &["c1", "c2", "c3"]));
        cache.add_topology(topology("auto", "dB", &["c10", "c20", "c30"]));
        assert_eq!(2, cache.topologies().len());

        assert_eq!(
            cache.topologies_for(&tokens(&["dA"]))[0].walk,
            ["c1", "c2", "c3"]
        );
        assert_eq!(
            cache.topologies_for(&tokens(&["dB"]))[0].walk,
            ["c10", "c20", "c30"]
        );
    }

    #[test]
    fn a_moved_card_does_not_drag_its_old_decks_topology_along() {
        let mut cache = AugmentCache::open(std::path::Path::new("unused.json"));
        cache.add_topology(topology("auto", "dA", &["c1", "c2"]));

        assert!(!cache.has_topology_for(&tokens(&["dB"])));
        assert!(cache.topologies_for(&tokens(&["dB"])).is_empty());
        assert_eq!(1, cache.topologies_for(&tokens(&["dA"])).len());
    }

    #[test]
    fn topologies_for_keeps_only_the_decks_own() {
        let mut cache = AugmentCache::open(std::path::Path::new("unused.json"));
        cache.add_topology(topology("architecture", "dA", &["c1", "c2", "c3"]));
        cache.add_topology(topology("capitals", "dB", &["c10", "c20", "c30"]));

        let mine = cache.topologies_for(&tokens(&["dA"]));
        assert_eq!(1, mine.len());
        assert_eq!("architecture", mine[0].name);

        assert!(cache.topologies_for(&tokens(&["dZ"])).is_empty());
    }

    #[test]
    fn has_topology_for_reports_presence_without_cross_deck_leak() {
        let mut cache = AugmentCache::open(std::path::Path::new("unused.json"));
        cache.add_topology(topology("architecture", "dA", &["c1", "c2", "c3"]));

        assert!(cache.has_topology_for(&tokens(&["dA"])));
        assert!(!cache.has_topology_for(&tokens(&["dZ"])));
    }

    #[test]
    fn region_path_locates_the_card_and_lists_regions_in_walk_order() {
        let t = topo_regions(vec![
            region("Parsing", &["c1", "c2"]),
            region("Session", &["c3", "c4"]),
            region("Persistence", &["c5"]),
        ]);
        let (names, current) = t.region_path("c3").unwrap();
        assert_eq!(vec!["Parsing", "Session", "Persistence"], names);
        assert_eq!(1, current);
    }

    #[test]
    fn region_cards_finds_by_name_case_insensitively() {
        let t = topo_regions(vec![
            region("Persistence", &["c10", "c20"]),
            region("Engine", &["c30"]),
        ]);
        assert_eq!(region_ids(&t, "persistence"), ["c10", "c20"]);
        assert_eq!(region_ids(&t, "Engine"), ["c30"]);
        assert!(t.region_cards("nope").is_none());
    }

    #[test]
    fn region_path_none_when_card_absent_or_no_regions() {
        let t = topo_regions(vec![region("A", &["c1"])]);
        assert!(t.region_path("c99").is_none());
        assert!(topo_regions(vec![]).region_path("c1").is_none());
    }

    #[test]
    fn topology_order_from_walk_ranks_present_and_misses_absent() {
        let walk = ["c10".to_string(), "c20".to_string(), "c30".to_string()];
        let order = TopologyOrder::from_walk(&walk);
        assert_eq!(Some(0), order.rank_of("c10"));
        assert_eq!(Some(2), order.rank_of("c30"));
        assert_eq!(None, order.rank_of("c99"));
    }

    fn plain_card(back: &str) -> Card {
        let mut c = Card::plain("deck.md".into(), "Q".into(), vec![back.into()], None, 1);
        let slug: String = back
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        c.token = Some(std::sync::Arc::from(format!("q{slug}").as_str()));
        c
    }

    fn cloze_card(back: &str) -> Card {
        let mut c = plain_card(back);
        c.hash_lines = Some(vec![back.into()]);
        c.hole = Some(0);
        c
    }

    fn topo_over(name: &str, deck_token: &str, card: &str) -> Topology {
        Topology {
            name: name.into(),
            principle: String::new(),
            edges: Vec::new(),
            walk: vec![card.into()],
            regions: Vec::new(),
            deck_token: deck_token.into(),
        }
    }

    fn cid(c: &Card) -> String {
        c.id().expect("test card is stamped")
    }

    #[test]
    fn summarize_counts_coverage_against_each_targets_eligible_cards() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        let cards = vec![
            plain_card("a"),
            plain_card("b"),
            plain_card("c"),
            cloze_card("z"),
        ];
        cache.set_distractors(
            &cid(&cards[0]),
            vec!["x".into()],
            cards[0].content_fingerprint,
        );
        cache.set_distractors(
            &cid(&cards[1]),
            vec!["y".into()],
            cards[1].content_fingerprint,
        );
        cache.set_note(&cid(&cards[0]), "n".into(), cards[0].content_fingerprint);
        cache.set_variants(
            &cid(&cards[0]),
            vec!["v".into()],
            cards[0].content_fingerprint,
        );
        cache.set_keypoints(
            &cid(&cards[2]),
            vec!["k1".into(), "k2".into()],
            cards[2].content_fingerprint,
        );
        cache.add_topology(topo_over("auto", "d1", &cid(&cards[0])));

        let s = cache.summarize(&cards, &tokens(&["d1"]));
        assert_eq!(
            Coverage {
                covered: 2,
                eligible: 4
            },
            s.choices
        );
        assert_eq!(
            Coverage {
                covered: 1,
                eligible: 4
            },
            s.notes
        );
        assert_eq!(
            Coverage {
                covered: 1,
                eligible: 3
            },
            s.questions
        );
        assert_eq!(
            Coverage {
                covered: 1,
                eligible: 4
            },
            s.keypoints
        );
        assert_eq!(vec!["auto".to_string()], s.topologies);
    }

    #[test]
    fn missing_returns_only_uncovered_eligible_cards() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        let cards = vec![plain_card("a"), plain_card("b"), cloze_card("z")];
        cache.set_distractors(
            &cid(&cards[0]),
            vec!["x".into()],
            cards[0].content_fingerprint,
        );

        let miss: Vec<String> = cache
            .missing_choices(&cards)
            .iter()
            .map(|w| w.id.clone())
            .collect();
        assert_eq!(miss, [cid(&cards[1]), cid(&cards[2])]);

        let mq: Vec<String> = cache
            .missing_questions(&cards)
            .iter()
            .map(|w| w.id.clone())
            .collect();
        assert_eq!(mq, [cid(&cards[0]), cid(&cards[1])]);
    }

    #[test]
    fn a_group_reads_only_at_its_own_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        cache.set_group("card-a", "g0".into(), FP);
        assert_eq!(Some("g0"), cache.group("card-a", FP));
        assert_eq!(
            None,
            cache.group("card-a", FP + 1),
            "an edited card reads ungrouped"
        );
        assert_eq!(None, cache.group("card-b", FP));
    }

    #[test]
    fn clearing_the_choices_target_drops_groups_with_the_lists() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        cache.set_group("card-a", "g0".into(), FP);
        cache.set_distractors("card-a", vec!["x".into()], FP);
        let ids: HashSet<String> = ["card-a".to_string()].into();
        cache.clear_distractors(&ids);
        assert_eq!(None, cache.group("card-a", FP));
        assert_eq!(None, cache.distractors("card-a", FP));
    }

    #[test]
    fn a_freshly_grouped_card_with_a_viable_pool_is_not_a_choices_gap() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        let cards: Vec<Card> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|s| {
                let mut c = plain_card(s);
                c.deck_id = std::sync::Arc::from("deck-x");
                c
            })
            .collect();
        for card in &cards[..4] {
            cache.set_group(&card.id().unwrap(), "g0".into(), card.content_fingerprint);
        }
        let missing: Vec<String> = cache
            .missing_choices(&cards)
            .iter()
            .map(|item| item.answer.clone())
            .collect();
        assert_eq!(
            ["e"],
            missing.as_slice(),
            "four grouped cards cover each other; the ungrouped fifth is the gap"
        );
    }

    #[test]
    fn a_card_with_authored_distractors_is_not_a_choices_gap() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AugmentCache::open(dir.path().join("deck1.json"));
        let mut authored = plain_card("a");
        authored.authored_distractors = vec!["x".into(), "y".into()];
        let cards = vec![authored, plain_card("b")];
        let missing: Vec<String> = cache
            .missing_choices(&cards)
            .iter()
            .map(|item| item.answer.clone())
            .collect();
        assert_eq!(["b"], missing.as_slice());

        let summary = cache.summarize(&cards, &HashSet::new());
        assert_eq!(1, summary.choices.covered);
        assert_eq!(2, summary.choices.eligible);
    }

    #[test]
    fn clear_distractors_is_deck_scoped_and_prunes_empty_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        let mine = plain_card("a");
        let other = plain_card("other-deck-card");
        cache.set_distractors(&cid(&mine), vec!["x".into()], mine.content_fingerprint);
        cache.set_distractors(&cid(&other), vec!["y".into()], other.content_fingerprint);

        let deck_ids: HashSet<String> = [cid(&mine)].into_iter().collect();
        cache.clear_distractors(&deck_ids);

        assert_eq!(
            None,
            cache.distractors(&cid(&mine), mine.content_fingerprint)
        );
        assert!(!cache.contains(&cid(&mine)));
        assert_eq!(
            Some(["y".to_string()].as_slice()),
            cache.distractors(&cid(&other), other.content_fingerprint)
        );
    }

    #[test]
    fn clear_notes_keeps_other_fields_and_does_not_prune() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        let c = plain_card("a");
        cache.set_note(&cid(&c), "n".into(), c.content_fingerprint);
        cache.set_distractors(&cid(&c), vec!["x".into()], c.content_fingerprint);

        let deck_ids: HashSet<String> = [cid(&c)].into_iter().collect();
        cache.clear_notes(&deck_ids);

        assert_eq!(None, cache.note(&cid(&c), c.content_fingerprint));
        assert_eq!(
            Some(["x".to_string()].as_slice()),
            cache.distractors(&cid(&c), c.content_fingerprint)
        );
        assert!(cache.contains(&cid(&c)));
    }

    #[test]
    fn remove_topology_is_name_and_deck_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        let mine = plain_card("a");
        let other = plain_card("other");
        cache.add_topology(topo_over("auto", "dA", &cid(&mine)));
        cache.add_topology(topo_over("auto", "dB", &cid(&other)));

        assert!(cache.remove_topology("auto", &tokens(&["dA"])));
        assert_eq!(1, cache.topologies().len());
        assert_eq!(1, cache.topologies_for(&tokens(&["dB"])).len());
        assert!(!cache.remove_topology("nope", &tokens(&["dA"])));
    }

    #[test]
    fn clear_all_removes_only_this_decks_augmentations() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        let mine = plain_card("a");
        let other = plain_card("other");
        cache.set_distractors(&cid(&mine), vec!["x".into()], mine.content_fingerprint);
        cache.set_note(&cid(&mine), "n".into(), mine.content_fingerprint);
        cache.add_topology(topo_over("auto", "dA", &cid(&mine)));
        cache.set_distractors(&cid(&other), vec!["y".into()], other.content_fingerprint);
        cache.add_topology(topo_over("auto", "dB", &cid(&other)));

        let deck_ids: HashSet<String> = [cid(&mine)].into_iter().collect();
        cache.clear_all(&deck_ids, &tokens(&["dA"]));

        assert!(!cache.contains(&cid(&mine)));
        assert!(cache.topologies_for(&tokens(&["dA"])).is_empty());
        assert_eq!(
            Some(["y".to_string()].as_slice()),
            cache.distractors(&cid(&other), other.content_fingerprint)
        );
        assert_eq!(1, cache.topologies_for(&tokens(&["dB"])).len());
    }

    #[test]
    fn apply_format_reshapes_display_without_changing_identity() {
        use std::sync::Arc;
        let mut card = Card::plain(
            Arc::from("d.md"),
            "List the parts".to_string(),
            vec!["A, B, C".to_string()],
            None,
            1,
        );
        card.token = Some(Arc::from("qfmt"));
        let id = cid(&card);
        let mut cache = AugmentCache::open(std::env::temp_dir().join("nonexistent-deck1.json"));
        cache.set_format(
            &id,
            Format {
                front: Some("Name the parts".to_string()),
                back: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                note: None,
                mode: Some(Mode::LineByLine),
            },
            card.content_fingerprint,
        );
        cache.apply_format(&mut card);
        assert_eq!(card.front, "Name the parts");
        assert_eq!(card.back_for_display(), ["A", "B", "C"]);
        assert_eq!(card.reveal, Some(Reveal::Line));
        assert_eq!(cid(&card), id);
    }

    #[test]
    fn apply_format_respects_an_explicit_reveal() {
        use std::sync::Arc;
        let mut card = Card::plain(Arc::from("d.md"), "f".into(), vec!["a".into()], None, 1);
        card.token = Some(Arc::from("qfmt2"));
        card.reveal = Some(Reveal::Flip);
        let id = cid(&card);
        let mut cache = AugmentCache::open(std::env::temp_dir().join("nonexistent-augment2.json"));
        cache.set_format(
            &id,
            Format {
                front: None,
                back: Vec::new(),
                note: None,
                mode: Some(Mode::LineByLine),
            },
            card.content_fingerprint,
        );
        cache.apply_format(&mut card);
        assert_eq!(card.reveal, Some(Reveal::Flip));
    }

    #[test]
    fn a_distractor_read_with_a_changed_fingerprint_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        cache.set_distractors("c1", vec!["w1".into(), "w2".into()], 100);
        assert_eq!(
            Some(["w1".to_string(), "w2".to_string()].as_slice()),
            cache.distractors("c1", 100)
        );
        assert_eq!(None, cache.distractors("c1", 200));
    }

    #[test]
    fn every_target_gates_on_its_own_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        cache.set_note("c1", "a fact".into(), 7);
        cache.set_variants("c1", vec!["v1".into()], 7);
        cache.set_keypoints("c1", vec!["k1".into()], 7);
        cache.set_format(
            "c1",
            Format {
                back: vec!["reshaped".into()],
                ..Default::default()
            },
            7,
        );
        assert!(cache.note("c1", 7).is_some());
        assert!(cache.variants("c1", 7).is_some());
        assert!(cache.keypoints("c1", 7).is_some());
        assert!(cache.format("c1", 7).is_some());
        assert_eq!(None, cache.note("c1", 8));
        assert!(cache.variants("c1", 8).is_none());
        assert!(cache.keypoints("c1", 8).is_none());
        assert!(cache.format("c1", 8).is_none());
    }

    #[test]
    fn an_entry_without_a_fingerprint_reads_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck1.json");
        std::fs::write(
            &path,
            r#"{"version":1,"cards":{"c1":{"distractors":["old"]}}}"#,
        )
        .unwrap();
        let cache = AugmentCache::open(&path);
        assert_eq!(None, cache.distractors("c1", 42));
    }

    #[test]
    fn a_stale_target_drops_out_of_coverage_and_into_the_gap_list() {
        let deck = crate::parser::parse_str(
            "d.md",
            "## q <!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n---\na\n",
        )
        .unwrap();
        let card = &deck[0];
        let id = card.id().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("deck1.json"));
        cache.set_format(
            &id,
            Format {
                back: vec!["reshaped".into()],
                ..Default::default()
            },
            card.content_fingerprint ^ 1,
        );
        let summary = cache.summarize(
            std::slice::from_ref(card),
            &std::collections::HashSet::new(),
        );
        assert_eq!(
            0, summary.format.covered,
            "a stale reshape must not count as covered"
        );
        assert_eq!(
            1,
            cache.missing_format(std::slice::from_ref(card)).len(),
            "it must resurface as a gap"
        );
    }
}
