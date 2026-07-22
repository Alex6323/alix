use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{answer::Mode, card::Card, depth::Reveal};

const CURRENT_VERSION: u32 = 1;

/// Display-only; never part of `Card::id()`, so applying it never touches progress.
/// An all-empty value still marks the card as checked, distinct from no cache entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl Augmentation {
    fn is_empty(&self) -> bool {
        self.distractors.is_empty()
            && self.note.is_none()
            && self.variants.is_empty()
            && self.keypoints.is_empty()
            && self.format.is_none()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
pub struct TopologyEdge {
    pub from: String,
    pub to: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
struct AugmentFile {
    #[serde(default = "default_version")]
    version: u32,
    cards: HashMap<String, Augmentation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    topologies: Vec<Topology>,
}

pub struct AugmentCache {
    path: PathBuf,
    cards: HashMap<String, Augmentation>,
    topologies: Vec<Topology>,
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
}

impl AugmentCache {
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let Loaded { cards, topologies } = load(&path).unwrap_or_default();
        Self {
            path,
            cards,
            topologies,
        }
    }

    pub fn save(&self) -> Result<(), AugmentError> {
        let io_err = |source| AugmentError::Io {
            path: self.path.clone(),
            source,
        };

        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(io_err)?;
        }

        let file = AugmentFile {
            version: CURRENT_VERSION,
            cards: self.cards.clone(),
            topologies: self.topologies.clone(),
        };
        let json = serde_json::to_string_pretty(&file).map_err(|source| AugmentError::Format {
            path: self.path.clone(),
            source,
        })?;

        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(io_err)?;
        std::fs::rename(&tmp, &self.path).map_err(io_err)?;
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
            match crate::token::parse_card_id(id) {
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

    /// Token-scoped, unlike [`clear_all`](Self::clear_all)'s exact ids, so a
    /// stale entry under a wiped token goes too. Does not save.
    pub fn wipe_tokens(
        &mut self,
        card_tokens: &HashSet<String>,
        deck_tokens: &HashSet<String>,
    ) -> bool {
        let cards_before = self.cards.len();
        self.cards.retain(|id, _| {
            !crate::token::parse_card_id(id)
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

#[derive(Default)]
struct Loaded {
    cards: HashMap<String, Augmentation>,
    topologies: Vec<Topology>,
}

fn load(path: &Path) -> Option<Loaded> {
    let text = std::fs::read_to_string(path).ok()?;
    let file: AugmentFile = serde_json::from_str(&text).ok()?;
    // A newer cache may hold a shape we'd mangle: ignore it instead of risking wrong options.
    if file.version > CURRENT_VERSION {
        return None;
    }
    Some(Loaded {
        cards: file.cards,
        topologies: file.topologies,
    })
}

fn default_version() -> u32 {
    1
}

pub fn augment_path_for(store_path: &Path) -> PathBuf {
    store_path.with_file_name("augment.json")
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
        let cache = AugmentCache::open(dir.path().join("augment.json"));
        assert!(cache.is_empty());
    }

    #[test]
    fn augment_entries_move_with_their_hole() {
        use crate::store::CascadeOutcome;
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
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
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
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
        let path = dir.path().join("augment.json");

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
    fn distractors_is_none_when_absent_or_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        assert_eq!(None, cache.distractors("c1", FP));
        cache.set_distractors("c1", Vec::new(), FP);
        assert_eq!(None, cache.distractors("c1", FP));
        assert!(cache.contains("c1"));
    }

    #[test]
    fn corrupt_file_yields_empty_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        std::fs::write(&path, "this is not json").unwrap();
        let cache = AugmentCache::open(&path);
        assert!(cache.is_empty());
    }

    #[test]
    fn newer_version_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        std::fs::write(&path, r#"{"version":999,"cards":{}}"#).unwrap();
        let cache = AugmentCache::open(&path);
        assert!(cache.is_empty());
    }

    #[test]
    fn every_string_key_loads_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        std::fs::write(
            &path,
            r#"{"version":1,"cards":{"not-a-token":{"distractors":["x"]},"q7":{"distractors":["y"]}}}"#,
        )
        .unwrap();
        let cache = AugmentCache::open(&path);
        assert_eq!(2, cache.len());
        assert!(cache.contains("q7"));
        assert!(cache.contains("not-a-token"));
        assert_eq!(None, cache.distractors("q7", FP));
        assert_eq!(None, cache.distractors("not-a-token", FP));
    }

    #[test]
    fn file_without_version_field_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        std::fs::write(&path, r#"{"cards":{"c3":{"distractors":["z"]}}}"#).unwrap();
        let cache = AugmentCache::open(&path);
        assert!(cache.contains("c3"));
        assert_eq!(None, cache.distractors("c3", FP));
    }

    #[test]
    fn augment_path_is_a_sibling_of_the_store() {
        let p = augment_path_for(Path::new("/data/alix/progress.json"));
        assert_eq!(Path::new("/data/alix/augment.json"), p);
    }

    #[test]
    fn set_distractors_replaces_previous() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
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
        let path = dir.path().join("augment.json");
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
        let path = dir.path().join("augment.json");
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
        let path = dir.path().join("augment.json");
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
        let path = dir.path().join("augment.json");
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
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
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
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
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
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
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
    fn a_card_with_authored_distractors_is_not_a_choices_gap() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AugmentCache::open(dir.path().join("augment.json"));
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
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
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
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
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
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
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
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
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
        let mut cache = AugmentCache::open(std::env::temp_dir().join("nonexistent-augment.json"));
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
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
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
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
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
    fn a_legacy_entry_without_a_fingerprint_reads_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
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
            "## q <!-- id: 4jkya9q3m8z0tw5v9y2b4n6d8f -->\n---\na\n",
        )
        .unwrap();
        let card = &deck[0];
        let id = card.id().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
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
