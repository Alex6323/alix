//! AI deck augmentation: a deliberate layer (`alix deck augment`) that lets an
//! LLM enrich a card's *presentation* without touching its identity or progress.
//! Three kinds, all generated up front and stored in one id-keyed cache, then
//! read at review time: choice-mode **distractors** (with the offline sampler in
//! [`crate::choice`] as fallback), a **note** (merged with the card's deck note
//! on reveal), and a pool of reworded question **variants** (a fresh one rotated
//! in as the front each time a card is shown). Each is an additive field on
//! [`Augmentation`].
//!
//! Alongside those per-card fields sit deck-level [`Topology`] entries (one or
//! more): each is a relational graph (labeled edges plus a suggested walk order)
//! over *all* the cards, so review can present them in a connected order ("same
//! module", "also in Europe") rather than at random. A deck can hold several —
//! one per organizing principle ("north to south" vs "by continent") — keyed by
//! the guidance that produced it. Being whole-deck rather than per-card, they
//! live beside the card map, not inside an [`Augmentation`]. (Experimental.)
//!
//! Everything here is **regenerable**, so the cache is best-effort: a missing,
//! corrupt, or future-versioned file just yields an empty cache rather than an
//! error — a bad cache must never block a review. It is keyed by the card's
//! identity hash, so editing a card's answer (which changes its id) is a cache
//! miss and silently regenerates, never serving stale options.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{answer::Mode, card::Card, depth::Reveal};

/// The on-disk cache-format version. Bumped only if the persisted shape changes
/// incompatibly; because the cache is regenerable, a newer version is ignored
/// (an empty cache is returned) rather than refused.
const CURRENT_VERSION: u32 = 1;

/// A tidied presentation for a badly-shaped card (e.g. an enumeration crammed
/// into one prose answer): reshaped display text for the front/answer/note and a
/// suggested answer mode. Display-only — never part of `Card::id()`, so it
/// preserves progress. Absent for a card already well-shaped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Format {
    /// Reshaped question (readability only). `None` keeps the card's front.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front: Option<String>,
    /// Reshaped answer, as display lines. Empty keeps the card's own back.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub back: Vec<String>,
    /// Reshaped note. `None` keeps the card's deck note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Suggested reveal-method (`flip` or `line`). Applied at review only if the
    /// card declares no `% reveal:` of its own (`flip`→flip, `line`→line). Depth
    /// modes (explain) aren't suggested — depth is derived now (spec §8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
}

/// Maps a suggested self-graded `Mode` onto the authored reveal axis: `flip` and
/// `line` have direct reveal equivalents; `explain` (and anything else) does not
/// — depth is derived now (spec §8), so an explain suggestion is dropped.
fn reveal_from_suggested(mode: Mode) -> Option<Reveal> {
    match mode {
        Mode::Flip => Some(Reveal::Flip),
        Mode::LineByLine => Some(Reveal::Line),
        _ => None,
    }
}

/// The AI-derived presentation data for a single card, keyed in the cache by the
/// card's identity hash. Fields are additive: new augmentation kinds (e.g.
/// morphed question variants) become new optional fields here so old caches keep
/// loading.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Augmentation {
    /// Generated wrong-answer options for choice mode, in no particular order.
    /// Empty when none were generated (the caller falls back to offline
    /// sampling).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub distractors: Vec<String>,
    /// A generated note (trivia / context / a mnemonic), merged with the card's
    /// own deck note on reveal. `None` when none was generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Reworded phrasings of the question (each keeping the same answer), one of
    /// which is shown in place of the card's front at review time so the card
    /// can't be answered by recognizing a fixed wording. Empty when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<String>,
    /// The load-bearing claims the card's answer makes, decomposed so Explain
    /// mode can check a reconstruction against them one by one (the grade is
    /// derived from how many are covered). Empty for an atomic answer that
    /// doesn't decompose — such a card keeps its plain self-graded reveal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keypoints: Vec<String>,
    /// A display-only reshape for a badly-shaped card (front/answer/note + a
    /// suggested mode), applied at review without touching the deck or the card's
    /// identity. `None` for a card already well-shaped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
}

impl Augmentation {
    /// Whether this card holds no augmentation of any kind — the state after the
    /// last field is cleared. Removal prunes such entries so the cache doesn't
    /// keep dead keys.
    fn is_empty(&self) -> bool {
        self.distractors.is_empty()
            && self.note.is_none()
            && self.variants.is_empty()
            && self.keypoints.is_empty()
            && self.format.is_none()
    }
}

/// A deck-level relational augmentation: an AI-derived graph over the cards plus
/// a suggested order to walk it, so review can move along the connective tissue
/// of the material instead of shuffling. Unlike [`Augmentation`] this is a
/// whole-deck object, so it sits beside the card map, not inside one entry. A
/// deck can hold several, each identified by its [`name`](Self::name).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topology {
    /// The selection handle: the `--with` guidance that produced this topology
    /// ("north to south"), or `"auto"` when none was given. Stable and
    /// user-chosen, so it's how a deck's several topologies are told apart and
    /// (later) which one the scheduler is asked to walk.
    pub name: String,
    /// The organizing principle the walk follows — what the model chose or
    /// articulated from the guidance. Shown so the order's rationale is legible
    /// ("why this next card"). Usually richer than [`name`](Self::name).
    pub principle: String,
    /// Directed, labeled edges between cards (by identity hash): `from` → `to`
    /// reads as "after `from`, `to` is a natural next step", and `label` says
    /// why ("calls into", "same continent").
    pub edges: Vec<TopologyEdge>,
    /// A suggested order to visit the cards (by identity hash) such that
    /// consecutive cards relate — the model's default walk of the graph.
    pub walk: Vec<u64>,
    /// Coarse named groupings of the cards (stages / themes), in the order the
    /// walk passes through them — the "where am I" map shown as a review
    /// breadcrumb. Deliberately high-level so a region name orients without
    /// revealing any card's answer. Additive: caches without it still load.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<TopologyRegion>,
}

/// One directed, labeled relationship between two cards in a [`Topology`]. Edges
/// power the walk order (and a future graph view); they are not shown during a
/// drill, since a label *into* a card tends to reveal that card's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyEdge {
    /// The card this edge leads *from* (its identity hash).
    pub from: u64,
    /// The card this edge leads *to* (its identity hash).
    pub to: u64,
    /// A short reason the two relate ("same module", "also in Europe").
    pub label: String,
}

/// A coarse, named group of cards in a [`Topology`] — one stage/theme of the
/// walk ("Parsing", "Persistence"). Its name is the orientation cue shown at
/// review time; being high-level, it situates without spoiling an answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyRegion {
    /// A short human place-name for the group (not a sentence).
    pub name: String,
    /// The cards in this region (by identity hash).
    pub cards: Vec<u64>,
}

impl Topology {
    /// The region breadcrumb for orienting on `card`: the region names in walk
    /// order plus the index of the region containing `card`, so a frontend can
    /// render a "…prev › **current** › next…" trail (windowed to fit its width).
    /// `None` when the topology has no regions or the card isn't in one.
    pub fn region_path(&self, card: u64) -> Option<(Vec<&str>, usize)> {
        let current = self.regions.iter().position(|r| r.cards.contains(&card))?;
        let names = self.regions.iter().map(|r| r.name.as_str()).collect();
        Some((names, current))
    }

    /// Whether this topology was built over the deck whose cards are `deck_ids` —
    /// true when its walk shares any card with the deck. Card ids embed the deck's
    /// file name, so they never collide across decks; this is what keeps one cache
    /// shared by several decks (decks sharing a store) from leaking one deck's
    /// topology onto another.
    pub fn covers(&self, deck_ids: &HashSet<u64>) -> bool {
        self.walk.iter().any(|id| deck_ids.contains(id))
    }

    /// The card ids of the region named `name` (matched case-insensitively), for
    /// focusing a session on one area. `None` when no region matches.
    pub fn region_cards(&self, name: &str) -> Option<&[u64]> {
        self.regions
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(name))
            .map(|r| r.cards.as_slice())
    }
}

/// A topology's walk projected to a session-ready lookup: each card id mapped to
/// its position in the walk, so a queue can be sorted by it. Cards absent from
/// the walk have no rank and sort to the end (keeping scheduler order). Lives
/// here, beside [`Topology`], so `session` only imports the type and stays free
/// of cache logic.
#[derive(Clone, Debug, Default)]
pub struct TopologyOrder {
    rank: HashMap<u64, usize>,
}

impl TopologyOrder {
    /// Builds the lookup from a topology's walk (card ids in walk order).
    pub fn from_walk(walk: &[u64]) -> Self {
        Self {
            rank: walk
                .iter()
                .copied()
                .enumerate()
                .map(|(i, id)| (id, i))
                .collect(),
        }
    }

    /// The card's position in the walk, or `None` if it isn't on the walk.
    pub fn rank_of(&self, card_id: u64) -> Option<usize> {
        self.rank.get(&card_id).copied()
    }
}

/// On-disk representation of the cache.
#[derive(Serialize, Deserialize)]
struct AugmentFile {
    /// Format version; defaults to the oldest for a file written before the
    /// field existed.
    #[serde(default = "default_version")]
    version: u32,
    /// Augmentations keyed by the decimal string of the card's identity hash
    /// (JSON object keys must be strings).
    cards: HashMap<String, Augmentation>,
    /// The deck-level topologies, one per organizing principle. Additive and
    /// defaulted so caches without it keep loading.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    topologies: Vec<Topology>,
}

/// A best-effort, id-keyed cache of AI augmentations for cards, plus an optional
/// deck-level [`Topology`].
pub struct AugmentCache {
    path: PathBuf,
    cards: HashMap<u64, Augmentation>,
    topologies: Vec<Topology>,
}

/// An error *saving* the cache. Loading never errors (see the module docs): a
/// cache that can't be read is simply treated as empty.
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
    /// Opens the cache at `path`. A missing, unreadable, malformed, or
    /// newer-than-understood file yields an empty cache — the data is
    /// regenerable, so a bad cache must never fail a review.
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let Loaded { cards, topologies } = load(&path).unwrap_or_default();
        Self {
            path,
            cards,
            topologies,
        }
    }

    /// Saves the cache atomically (write to a temp file, then rename).
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
            cards: self
                .cards
                .iter()
                .map(|(id, aug)| (id.to_string(), aug.clone()))
                .collect(),
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

    /// The cached distractors for a card, or `None` when absent or empty — so
    /// the caller can fall back to offline sampling with a single check.
    pub fn distractors(&self, card_id: u64) -> Option<&[String]> {
        self.cards
            .get(&card_id)
            .map(|aug| aug.distractors.as_slice())
            .filter(|d| !d.is_empty())
    }

    /// Whether an augmentation is already cached for a card. Used to warm only
    /// the cards that still need it.
    pub fn contains(&self, card_id: u64) -> bool {
        self.cards.contains_key(&card_id)
    }

    /// Stores the distractors for a card, replacing any previous ones. Does not
    /// save.
    pub fn set_distractors(&mut self, card_id: u64, distractors: Vec<String>) {
        self.cards.entry(card_id).or_default().distractors = distractors;
    }

    /// The cached note for a card, if any.
    pub fn note(&self, card_id: u64) -> Option<&str> {
        self.cards.get(&card_id).and_then(|aug| aug.note.as_deref())
    }

    /// Stores a generated note for a card, replacing any previous one. Does not
    /// save.
    pub fn set_note(&mut self, card_id: u64, note: String) {
        self.cards.entry(card_id).or_default().note = Some(note);
    }

    /// The cached presentation reshape for a card, if any.
    pub fn format(&self, card_id: u64) -> Option<&Format> {
        self.cards.get(&card_id).and_then(|aug| aug.format.as_ref())
    }

    /// Caches a presentation reshape for a card.
    pub fn set_format(&mut self, card_id: u64, format: Format) {
        self.cards.entry(card_id).or_default().format = Some(format);
    }

    /// The cached question variants for a card (a pool to rotate through), or
    /// `None` when absent or empty.
    pub fn variants(&self, card_id: u64) -> Option<&[String]> {
        self.cards
            .get(&card_id)
            .map(|aug| aug.variants.as_slice())
            .filter(|v| !v.is_empty())
    }

    /// Picks a question phrasing for a card from the pool of the authored
    /// `original` plus the cached variants, rotating by `seed` (a plain modulo).
    /// The original sits at index 0, so it stays in the rotation. `None` when no
    /// variants are cached — the caller then keeps the original front unchanged.
    pub fn pick_front(&self, card_id: u64, original: &str, seed: u64) -> Option<String> {
        let variants = self.variants(card_id)?;
        let pool_len = variants.len() + 1; // + the original at index 0
        let idx = (seed % pool_len as u64) as usize;
        Some(if idx == 0 {
            original.to_string()
        } else {
            variants[idx - 1].clone()
        })
    }

    /// Stores the question variants for a card, replacing any previous ones.
    /// Does not save.
    pub fn set_variants(&mut self, card_id: u64, variants: Vec<String>) {
        self.cards.entry(card_id).or_default().variants = variants;
    }

    /// The cached key points for a card (the Explain-mode checklist rubric), or
    /// `None` when absent or empty — so the caller can fall back to the plain
    /// self-graded reveal with one check.
    pub fn keypoints(&self, card_id: u64) -> Option<&[String]> {
        self.cards
            .get(&card_id)
            .map(|aug| aug.keypoints.as_slice())
            .filter(|k| !k.is_empty())
    }

    /// Stores the key points for a card, replacing any previous ones. Does not
    /// save.
    pub fn set_keypoints(&mut self, card_id: u64, keypoints: Vec<String>) {
        self.cards.entry(card_id).or_default().keypoints = keypoints;
    }

    /// All cached deck-level topologies, one per organizing principle.
    pub fn topologies(&self) -> &[Topology] {
        &self.topologies
    }

    /// The cached topologies belonging to the deck whose cards are `deck_ids`. A
    /// cache file is shared by every deck that shares a store (e.g. all loose
    /// decks under the global store), so this filters out other decks' topologies
    /// by card membership — see [`Topology::covers`].
    pub fn topologies_for(&self, deck_ids: &HashSet<u64>) -> Vec<&Topology> {
        self.topologies
            .iter()
            .filter(|t| t.covers(deck_ids))
            .collect()
    }

    /// Whether any cached topology covers the deck whose cards are `deck_ids` —
    /// i.e. the picker's focus drawer would open for it. Cheaper than
    /// [`topologies_for`](Self::topologies_for) when only presence matters (no
    /// allocation).
    pub fn has_topology_for(&self, deck_ids: &HashSet<u64>) -> bool {
        self.topologies.iter().any(|t| t.covers(deck_ids))
    }

    /// The cached topology with the given [`name`](Topology::name), if any — how
    /// the scheduler selects which one to walk.
    pub fn topology(&self, name: &str) -> Option<&Topology> {
        self.topologies.iter().find(|t| t.name == name)
    }

    /// Stores a topology. It replaces an existing one with the same
    /// [`name`](Topology::name) **only when it's the same deck's** — its walk
    /// overlaps, so re-running a principle refreshes it — and otherwise appends.
    /// The deck check matters because a cache can be shared by several decks (one
    /// store): a like-named topology from another deck (e.g. both defaulting to
    /// `auto`) must not be clobbered. Does not save.
    pub fn add_topology(&mut self, topology: Topology) {
        let ids: HashSet<u64> = topology.walk.iter().copied().collect();
        match self
            .topologies
            .iter_mut()
            .find(|t| t.name == topology.name && t.covers(&ids))
        {
            Some(existing) => *existing = topology,
            None => self.topologies.push(topology),
        }
    }

    /// The number of cards with cached augmentations.
    pub fn len(&self) -> usize {
        self.cards.len()
    }

    /// Returns `true` if nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// What this deck's cache currently holds, per target — the data the Augment
    /// screen renders. Scoped to `cards` (this deck): the cache may be shared by
    /// other decks on the same store. Per-card targets report `(covered,
    /// eligible)`; topology is the list of this deck's topology names.
    pub fn summarize(&self, cards: &[Card]) -> CoverageSummary {
        let coverage = |eligible: &[&Card], covered: &dyn Fn(u64) -> bool| Coverage {
            covered: eligible.iter().filter(|c| covered(c.id())).count(),
            eligible: eligible.len(),
        };
        let all: Vec<&Card> = cards.iter().collect();
        let plain: Vec<&Card> = cards.iter().filter(|c| c.hash_lines.is_none()).collect();
        let deck_ids: HashSet<u64> = cards.iter().map(Card::id).collect();
        CoverageSummary {
            choices: coverage(&all, &|id| self.distractors(id).is_some()),
            notes: coverage(&all, &|id| self.note(id).is_some()),
            questions: coverage(&plain, &|id| self.variants(id).is_some()),
            keypoints: coverage(&all, &|id| self.keypoints(id).is_some()),
            format: coverage(&plain, &|id| self.format(id).is_some()),
            topologies: self
                .topologies_for(&deck_ids)
                .iter()
                .map(|t| t.name.clone())
                .collect(),
        }
    }

    /// The eligible cards a target doesn't yet cover, as generation-ready items —
    /// the **fill-the-gaps** input. `eligible` mirrors each generator's own filter
    /// (e.g. plain-only for question variants); `covered` is its cache getter.
    ///
    /// A card a generator legitimately omits (no usable distractor, an atomic
    /// answer) stays "missing", so a later fill-the-gaps will re-attempt it. That's
    /// accepted: generation is explicit and costed, and `--overwrite` (regenerate
    /// all) is the deliberate alternative.
    fn missing(
        &self,
        cards: &[Card],
        eligible: impl Fn(&Card) -> bool,
        covered: impl Fn(u64) -> bool,
    ) -> Vec<WarmItem> {
        cards
            .iter()
            .filter(|c| eligible(c) && !covered(c.id()))
            .map(WarmItem::from_card)
            .collect()
    }

    /// Cards still missing choice distractors (all cards eligible).
    pub fn missing_choices(&self, cards: &[Card]) -> Vec<WarmItem> {
        self.missing(cards, |_| true, |id| self.distractors(id).is_some())
    }

    /// Cards still missing a note (all cards eligible).
    pub fn missing_notes(&self, cards: &[Card]) -> Vec<WarmItem> {
        self.missing(cards, |_| true, |id| self.note(id).is_some())
    }

    /// Plain cards still missing question variants (cloze cards are ineligible).
    pub fn missing_questions(&self, cards: &[Card]) -> Vec<WarmItem> {
        self.missing(
            cards,
            |c| c.hash_lines.is_none(),
            |id| self.variants(id).is_some(),
        )
    }

    /// Cards still missing Explain-mode key points (all cards eligible).
    pub fn missing_keypoints(&self, cards: &[Card]) -> Vec<WarmItem> {
        self.missing(cards, |_| true, |id| self.keypoints(id).is_some())
    }

    /// Plain cards (cloze excluded) that have no cached reshape yet.
    pub fn missing_format(&self, cards: &[Card]) -> Vec<WarmItem> {
        self.missing(
            cards,
            |c| c.hash_lines.is_none(),
            |id| self.format(id).is_some(),
        )
    }

    /// Removes this deck's cached distractors — only the cards in `deck_ids`,
    /// since the cache may be shared with other decks — pruning any entry left
    /// empty. Does not save.
    pub fn clear_distractors(&mut self, deck_ids: &HashSet<u64>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.distractors.clear();
            }
        }
        self.prune_empty(deck_ids);
    }

    /// Removes this deck's cached notes (see [`clear_distractors`](Self::clear_distractors)).
    pub fn clear_notes(&mut self, deck_ids: &HashSet<u64>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.note = None;
            }
        }
        self.prune_empty(deck_ids);
    }

    /// Removes this deck's cached question variants (see
    /// [`clear_distractors`](Self::clear_distractors)).
    pub fn clear_variants(&mut self, deck_ids: &HashSet<u64>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.variants.clear();
            }
        }
        self.prune_empty(deck_ids);
    }

    /// Removes this deck's cached key points (see [`clear_distractors`](Self::clear_distractors)).
    pub fn clear_keypoints(&mut self, deck_ids: &HashSet<u64>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.keypoints.clear();
            }
        }
        self.prune_empty(deck_ids);
    }

    /// Removes cached reshapes for this deck, then prunes empty entries.
    pub fn clear_format(&mut self, deck_ids: &HashSet<u64>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.format = None;
            }
        }
        self.prune_empty(deck_ids);
    }

    /// Applies a cached presentation reshape to `card` for display: overwrites the
    /// (un-hashed) front and re-renders the deck note, sets the display-only
    /// `display_back` for the answer, and fills the reveal-method only if the card
    /// declares none. Never touches `card.back`, so `card.id()` is unchanged. A
    /// no-op when the card has no cached reshape.
    pub fn apply_format(&self, card: &mut Card) {
        let Some(fmt) = self.format(card.id()) else {
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

    /// Drops any of `deck_ids`' entries that no longer hold any augmentation, so
    /// clearing the last field leaves no dead key behind.
    fn prune_empty(&mut self, deck_ids: &HashSet<u64>) {
        for id in deck_ids {
            if self.cards.get(id).is_some_and(Augmentation::is_empty) {
                self.cards.remove(id);
            }
        }
    }

    /// Removes the named topology if it belongs to this deck (its name matches
    /// **and** it [`covers`](Topology::covers) `deck_ids`, so a like-named
    /// topology from another deck on a shared store is left alone). Returns
    /// whether one was removed. Does not save.
    pub fn remove_topology(&mut self, name: &str, deck_ids: &HashSet<u64>) -> bool {
        let before = self.topologies.len();
        self.topologies
            .retain(|t| !(t.name == name && t.covers(deck_ids)));
        self.topologies.len() != before
    }

    /// Removes every augmentation this deck owns — all per-card fields for
    /// `deck_ids` and all topologies covering them (the "remove all" action).
    /// Other decks sharing the cache are untouched. Does not save.
    pub fn clear_all(&mut self, deck_ids: &HashSet<u64>) {
        for id in deck_ids {
            self.cards.remove(id);
        }
        self.topologies.retain(|t| !t.covers(deck_ids));
    }
}

/// One per-card target's coverage for a deck: how many eligible cards already
/// have it cached. `eligible` is the denominator (e.g. plain-only for question
/// variants), `covered` the numerator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coverage {
    pub covered: usize,
    pub eligible: usize,
}

/// What a deck's augmentation cache currently holds, per target — the data the
/// Augment screen renders. Per-card targets are a [`Coverage`]; topology is the
/// list of the deck's topology names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageSummary {
    pub choices: Coverage,
    pub notes: Coverage,
    pub questions: Coverage,
    pub keypoints: Coverage,
    pub format: Coverage,
    pub topologies: Vec<String>,
}

/// The decoded contents of a cache file: the per-card map plus the deck-level
/// topology.
#[derive(Default)]
struct Loaded {
    cards: HashMap<u64, Augmentation>,
    topologies: Vec<Topology>,
}

/// Loads the cache, returning `None` on any problem (missing/corrupt/newer file)
/// so [`AugmentCache::open`] can fall back to empty.
fn load(path: &Path) -> Option<Loaded> {
    let text = std::fs::read_to_string(path).ok()?;
    let file: AugmentFile = serde_json::from_str(&text).ok()?;
    // A cache from a newer alix may hold a shape we'd mangle — ignore it and
    // regenerate rather than risk serving wrong options.
    if file.version > CURRENT_VERSION {
        return None;
    }
    let mut cards = HashMap::with_capacity(file.cards.len());
    for (key, aug) in file.cards {
        // Skip any key that isn't a u64 hash rather than discarding the whole
        // cache for one bad entry.
        if let Ok(id) = key.parse::<u64>() {
            cards.insert(id, aug);
        }
    }
    Some(Loaded {
        cards,
        topologies: file.topologies,
    })
}

/// Serde default for a cache file with no `version` field: the oldest format.
fn default_version() -> u32 {
    1
}

/// The cache path co-located with a given progress store, so the augmentations
/// live next to whatever store the review path uses (honoring `--store` and any
/// future per-workspace store).
pub fn augment_path_for(store_path: &Path) -> PathBuf {
    store_path.with_file_name("augment.json")
}

/// One card to generate an augmentation for.
#[derive(Clone, Debug)]
pub struct WarmItem {
    /// The card's identity hash (the cache key).
    pub id: u64,
    /// The question shown to the learner (the card front).
    pub question: String,
    /// The correct answer the augmentation must respect.
    pub answer: String,
    /// The card's deck note, if any — used by the format target to re-render it.
    pub note: Option<String>,
}

impl WarmItem {
    /// Builds the generation input for a card: its identity hash, its front, its
    /// joined back, and its deck note.
    pub fn from_card(card: &Card) -> Self {
        Self {
            id: card.id(),
            question: card.front.clone(),
            answer: card.back.join("\n"),
            note: card.note.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_from_suggested_maps_only_flip_and_line() {
        assert_eq!(Some(Reveal::Flip), reveal_from_suggested(Mode::Flip));
        assert_eq!(Some(Reveal::Line), reveal_from_suggested(Mode::LineByLine));
        // Depth modes (and anything else) have no reveal-axis equivalent.
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
    fn save_and_reload_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");

        let mut cache = AugmentCache::open(&path);
        cache.set_distractors(42, vec!["wrong a".into(), "wrong b".into()]);
        cache.save().unwrap();

        let reloaded = AugmentCache::open(&path);
        assert_eq!(1, reloaded.len());
        assert_eq!(
            Some(["wrong a".to_string(), "wrong b".to_string()].as_slice()),
            reloaded.distractors(42)
        );
    }

    #[test]
    fn distractors_is_none_when_absent_or_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        assert_eq!(None, cache.distractors(1)); // absent
        cache.set_distractors(1, Vec::new());
        assert_eq!(None, cache.distractors(1)); // present but empty
        assert!(cache.contains(1));
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
    fn a_single_bad_key_does_not_drop_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        std::fs::write(
            &path,
            r#"{"version":1,"cards":{"not-a-number":{"distractors":["x"]},"7":{"distractors":["y"]}}}"#,
        )
        .unwrap();
        let cache = AugmentCache::open(&path);
        assert_eq!(1, cache.len());
        assert_eq!(Some(["y".to_string()].as_slice()), cache.distractors(7));
    }

    #[test]
    fn file_without_version_field_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        std::fs::write(&path, r#"{"cards":{"3":{"distractors":["z"]}}}"#).unwrap();
        let cache = AugmentCache::open(&path);
        assert_eq!(Some(["z".to_string()].as_slice()), cache.distractors(3));
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
        cache.set_distractors(1, vec!["old".into()]);
        cache.set_distractors(1, vec!["new a".into(), "new b".into()]);
        assert_eq!(
            Some(["new a".to_string(), "new b".to_string()].as_slice()),
            cache.distractors(1)
        );
    }

    #[test]
    fn note_roundtrips_through_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        let mut cache = AugmentCache::open(&path);
        cache.set_note(7, "a memorable fact".into());
        cache.save().unwrap();
        let reloaded = AugmentCache::open(&path);
        assert_eq!(Some("a memorable fact"), reloaded.note(7));
        assert_eq!(None, reloaded.note(8));
    }

    #[test]
    fn variants_roundtrip_and_pick() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        let mut cache = AugmentCache::open(&path);
        cache.set_variants(5, vec!["one".into(), "two".into(), "three".into()]);
        cache.save().unwrap();
        let reloaded = AugmentCache::open(&path);
        assert_eq!(3, reloaded.variants(5).unwrap().len());
        // pool = [original] + 3 variants = 4; idx = seed % 4, original at 0.
        assert_eq!(Some("ORIG".to_string()), reloaded.pick_front(5, "ORIG", 0));
        assert_eq!(Some("one".to_string()), reloaded.pick_front(5, "ORIG", 1));
        assert_eq!(Some("ORIG".to_string()), reloaded.pick_front(5, "ORIG", 4)); // 4 % 4 == 0
        assert_eq!(None, reloaded.pick_front(6, "ORIG", 0)); // no variants
    }

    #[test]
    fn keypoints_roundtrip_through_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        let mut cache = AugmentCache::open(&path);
        cache.set_keypoints(9, vec!["claim a".into(), "claim b".into()]);
        cache.save().unwrap();
        let reloaded = AugmentCache::open(&path);
        assert_eq!(
            Some(["claim a".to_string(), "claim b".to_string()].as_slice()),
            reloaded.keypoints(9)
        );
        assert_eq!(None, reloaded.keypoints(10)); // none cached
    }

    fn topology(name: &str, walk: Vec<u64>) -> Topology {
        Topology {
            name: name.into(),
            principle: format!("principle for {name}"),
            edges: vec![TopologyEdge {
                from: walk[0],
                to: walk[1],
                label: "x".into(),
            }],
            walk,
            regions: Vec::new(),
        }
    }

    fn region(name: &str, cards: Vec<u64>) -> TopologyRegion {
        TopologyRegion {
            name: name.into(),
            cards,
        }
    }

    fn topo_regions(regions: Vec<TopologyRegion>) -> Topology {
        Topology {
            name: "n".into(),
            principle: "p".into(),
            edges: Vec::new(),
            walk: Vec::new(),
            regions,
        }
    }

    #[test]
    fn topology_roundtrips_through_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        let mut cache = AugmentCache::open(&path);
        assert!(cache.topologies().is_empty());
        cache.add_topology(topology("auto", vec![1, 2]));
        cache.save().unwrap();

        let reloaded = AugmentCache::open(&path);
        let t = reloaded.topology("auto").unwrap();
        assert_eq!("principle for auto", t.principle);
        assert_eq!(vec![1, 2], t.walk);
        assert_eq!(1, t.edges.len());
    }

    #[test]
    fn add_topology_appends_new_names_and_replaces_same_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        cache.add_topology(topology("north to south", vec![1, 2]));
        cache.add_topology(topology("by continent", vec![3, 4]));
        assert_eq!(2, cache.topologies().len());

        // Re-running the same principle over the same deck (the walk still covers
        // its cards) refreshes it in place, not appends.
        cache.add_topology(topology("north to south", vec![1, 2, 7]));
        assert_eq!(2, cache.topologies().len());
        assert_eq!(
            vec![1, 2, 7],
            cache.topology("north to south").unwrap().walk
        );
        assert_eq!(vec![3, 4], cache.topology("by continent").unwrap().walk);
        assert!(cache.topology("alphabetical").is_none());
    }

    #[test]
    fn add_topology_keeps_like_named_topologies_for_different_decks() {
        // Two decks sharing a store both default to the name `auto`; their walks
        // are disjoint (different decks), so the second must NOT clobber the first.
        let mut cache = AugmentCache::open(std::path::Path::new("unused.json"));
        cache.add_topology(topology("auto", vec![1, 2, 3])); // deck A
        cache.add_topology(topology("auto", vec![10, 20, 30])); // deck B
        assert_eq!(2, cache.topologies().len());

        let a: HashSet<u64> = [1, 2, 3].into_iter().collect();
        let b: HashSet<u64> = [10, 20, 30].into_iter().collect();
        assert_eq!(vec![1, 2, 3], cache.topologies_for(&a)[0].walk);
        assert_eq!(vec![10, 20, 30], cache.topologies_for(&b)[0].walk);
    }

    #[test]
    fn topologies_for_keeps_only_the_decks_own() {
        // One cache shared by two decks (they share a store): each deck's cards
        // have disjoint ids, so `topologies_for` must return only the topology
        // whose walk overlaps the asked-for deck — no cross-deck leak.
        let mut cache = AugmentCache::open(std::path::Path::new("unused.json"));
        cache.add_topology(topology("architecture", vec![1, 2, 3]));
        cache.add_topology(topology("capitals", vec![10, 20, 30]));

        let arch: HashSet<u64> = [1, 2, 3].into_iter().collect();
        let mine = cache.topologies_for(&arch);
        assert_eq!(1, mine.len());
        assert_eq!("architecture", mine[0].name);

        // A deck sharing the store but with none of these cards sees no topology.
        let other: HashSet<u64> = [99].into_iter().collect();
        assert!(cache.topologies_for(&other).is_empty());
    }

    #[test]
    fn has_topology_for_reports_presence_without_cross_deck_leak() {
        let mut cache = AugmentCache::open(std::path::Path::new("unused.json"));
        cache.add_topology(topology("architecture", vec![1, 2, 3]));

        let mine: HashSet<u64> = [1, 2, 3].into_iter().collect();
        assert!(cache.has_topology_for(&mine));

        // A deck sharing the store but with none of these cards has no drawer.
        let other: HashSet<u64> = [99].into_iter().collect();
        assert!(!cache.has_topology_for(&other));
    }

    #[test]
    fn region_path_locates_the_card_and_lists_regions_in_walk_order() {
        let t = topo_regions(vec![
            region("Parsing", vec![1, 2]),
            region("Session", vec![3, 4]),
            region("Persistence", vec![5]),
        ]);
        let (names, current) = t.region_path(3).unwrap();
        assert_eq!(vec!["Parsing", "Session", "Persistence"], names);
        assert_eq!(1, current); // card 3 lives in "Session"
    }

    #[test]
    fn region_cards_finds_by_name_case_insensitively() {
        let t = topo_regions(vec![
            region("Persistence", vec![10, 20]),
            region("Engine", vec![30]),
        ]);
        assert_eq!(Some([10, 20].as_slice()), t.region_cards("persistence"));
        assert_eq!(Some([30].as_slice()), t.region_cards("Engine"));
        assert_eq!(None, t.region_cards("nope"));
    }

    #[test]
    fn region_path_none_when_card_absent_or_no_regions() {
        let t = topo_regions(vec![region("A", vec![1])]);
        assert!(t.region_path(99).is_none()); // card in no region
        assert!(topo_regions(vec![]).region_path(1).is_none()); // no regions at all
    }

    #[test]
    fn topology_order_from_walk_ranks_present_and_misses_absent() {
        let order = TopologyOrder::from_walk(&[10, 20, 30]);
        assert_eq!(Some(0), order.rank_of(10));
        assert_eq!(Some(2), order.rank_of(30));
        assert_eq!(None, order.rank_of(99));
    }

    // ── Coverage / gaps / removal (the web Augment screen's lib backing) ──

    fn plain_card(back: &str) -> Card {
        Card::plain("deck.txt".into(), "Q".into(), vec![back.into()], None, 1)
    }

    fn cloze_card(back: &str) -> Card {
        let mut c = plain_card(back);
        c.hash_lines = Some(vec![back.into()]);
        c
    }

    fn topo_over(name: &str, card: u64) -> Topology {
        Topology {
            name: name.into(),
            principle: String::new(),
            edges: Vec::new(),
            walk: vec![card],
            regions: Vec::new(),
        }
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
        cache.set_distractors(cards[0].id(), vec!["x".into()]);
        cache.set_distractors(cards[1].id(), vec!["y".into()]);
        cache.set_note(cards[0].id(), "n".into());
        cache.set_variants(cards[0].id(), vec!["v".into()]);
        cache.set_keypoints(cards[2].id(), vec!["k1".into(), "k2".into()]);
        cache.add_topology(topo_over("auto", cards[0].id()));

        let s = cache.summarize(&cards);
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
        // Question variants are plain-only, so the cloze card is out of the denominator.
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
        cache.set_distractors(cards[0].id(), vec!["x".into()]);

        let miss: Vec<u64> = cache.missing_choices(&cards).iter().map(|w| w.id).collect();
        assert_eq!(vec![cards[1].id(), cards[2].id()], miss); // a covered; b + z still need it

        // Questions exclude cloze cards entirely, covered or not.
        let mq: Vec<u64> = cache
            .missing_questions(&cards)
            .iter()
            .map(|w| w.id)
            .collect();
        assert_eq!(vec![cards[0].id(), cards[1].id()], mq);
    }

    #[test]
    fn clear_distractors_is_deck_scoped_and_prunes_empty_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        let mine = plain_card("a");
        let other = plain_card("other-deck-card");
        cache.set_distractors(mine.id(), vec!["x".into()]);
        cache.set_distractors(other.id(), vec!["y".into()]);

        let deck_ids: HashSet<u64> = [mine.id()].into_iter().collect();
        cache.clear_distractors(&deck_ids);

        assert_eq!(None, cache.distractors(mine.id()));
        assert!(!cache.contains(mine.id())); // held nothing else → pruned
        // The other deck sharing this cache is untouched.
        assert_eq!(
            Some(["y".to_string()].as_slice()),
            cache.distractors(other.id())
        );
    }

    #[test]
    fn clear_notes_keeps_other_fields_and_does_not_prune() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        let c = plain_card("a");
        cache.set_note(c.id(), "n".into());
        cache.set_distractors(c.id(), vec!["x".into()]);

        let deck_ids: HashSet<u64> = [c.id()].into_iter().collect();
        cache.clear_notes(&deck_ids);

        assert_eq!(None, cache.note(c.id()));
        assert_eq!(
            Some(["x".to_string()].as_slice()),
            cache.distractors(c.id())
        );
        assert!(cache.contains(c.id())); // still has distractors → not pruned
    }

    #[test]
    fn remove_topology_is_name_and_deck_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        let mine = plain_card("a");
        let other = plain_card("other");
        cache.add_topology(topo_over("auto", mine.id()));
        cache.add_topology(topo_over("auto", other.id())); // same name, other deck

        let deck_ids: HashSet<u64> = [mine.id()].into_iter().collect();
        assert!(cache.remove_topology("auto", &deck_ids));
        assert_eq!(1, cache.topologies().len());
        let other_ids: HashSet<u64> = [other.id()].into_iter().collect();
        assert_eq!(1, cache.topologies_for(&other_ids).len()); // the other deck's survives
        assert!(!cache.remove_topology("nope", &deck_ids)); // no match → false
    }

    #[test]
    fn clear_all_removes_only_this_decks_augmentations() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        let mine = plain_card("a");
        let other = plain_card("other");
        cache.set_distractors(mine.id(), vec!["x".into()]);
        cache.set_note(mine.id(), "n".into());
        cache.add_topology(topo_over("auto", mine.id()));
        cache.set_distractors(other.id(), vec!["y".into()]);
        cache.add_topology(topo_over("auto", other.id()));

        let deck_ids: HashSet<u64> = [mine.id()].into_iter().collect();
        cache.clear_all(&deck_ids);

        assert!(!cache.contains(mine.id()));
        assert!(cache.topologies_for(&deck_ids).is_empty());
        // The other deck is intact.
        assert_eq!(
            Some(["y".to_string()].as_slice()),
            cache.distractors(other.id())
        );
        let other_ids: HashSet<u64> = [other.id()].into_iter().collect();
        assert_eq!(1, cache.topologies_for(&other_ids).len());
    }

    #[test]
    fn apply_format_reshapes_display_without_changing_identity() {
        use std::sync::Arc;
        let mut card = Card::plain(
            Arc::from("d.txt"),
            "List the parts".to_string(),
            vec!["A, B, C".to_string()],
            None,
            1,
        );
        let id = card.id();
        let mut cache = AugmentCache::open(std::env::temp_dir().join("nonexistent-augment.json"));
        cache.set_format(
            id,
            Format {
                front: Some("Name the parts".to_string()),
                back: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                note: None,
                mode: Some(Mode::LineByLine),
            },
        );
        cache.apply_format(&mut card);
        assert_eq!(card.front, "Name the parts");
        assert_eq!(card.back_for_display(), ["A", "B", "C"]);
        // The suggested `line` mode is applied as the `line` reveal-method.
        assert_eq!(card.reveal, Some(Reveal::Line));
        assert_eq!(card.id(), id); // identity preserved
    }

    #[test]
    fn apply_format_respects_an_explicit_reveal() {
        use std::sync::Arc;
        let mut card = Card::plain(Arc::from("d.txt"), "f".into(), vec!["a".into()], None, 1);
        card.reveal = Some(Reveal::Flip); // user's explicit choice
        let id = card.id();
        let mut cache = AugmentCache::open(std::env::temp_dir().join("nonexistent-augment2.json"));
        cache.set_format(
            id,
            Format {
                front: None,
                back: Vec::new(),
                note: None,
                mode: Some(Mode::LineByLine),
            },
        );
        cache.apply_format(&mut card);
        assert_eq!(card.reveal, Some(Reveal::Flip)); // suggestion does not override
    }
}
