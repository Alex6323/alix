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
//! identity token, so a card whose token was re-stamped (a cache miss) silently
//! regenerates, never serving stale options.

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
/// preserves progress. An all-empty Format marks a card the formatter checked
/// and left as-is, so it still counts as covered.
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
    /// Directed, labeled edges between cards (by card id): `from` → `to`
    /// reads as "after `from`, `to` is a natural next step", and `label` says
    /// why ("calls into", "same continent").
    pub edges: Vec<TopologyEdge>,
    /// A suggested order to visit the cards (by card id) such that
    /// consecutive cards relate — the model's default walk of the graph.
    pub walk: Vec<String>,
    /// Coarse named groupings of the cards (stages / themes), in the order the
    /// walk passes through them — the "where am I" map shown as a review
    /// breadcrumb. Deliberately high-level so a region name orients without
    /// revealing any card's answer. Additive: caches without it still load.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<TopologyRegion>,
    /// The identity token of the deck this topology was generated over — the
    /// stable owner. A card that later moves to another deck keeps its own
    /// token, but the topology stays bound to the deck it was built for, so it
    /// never leaks onto the new deck. This is how a cache shared by several decks
    /// (one store) tells one deck's topologies from another's. Defaulted so a
    /// topology written before this field loads as unowned (empty token).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub deck_token: String,
}

/// One directed, labeled relationship between two cards in a [`Topology`]. Edges
/// power the walk order (and a future graph view); they are not shown during a
/// drill, since a label *into* a card tends to reveal that card's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyEdge {
    /// The card this edge leads *from* (its card id).
    pub from: String,
    /// The card this edge leads *to* (its card id).
    pub to: String,
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
    /// The cards in this region (by card id).
    pub cards: Vec<String>,
}

impl Topology {
    /// The region breadcrumb for orienting on `card`: the region names in walk
    /// order plus the index of the region containing `card`, so a frontend can
    /// render a "…prev › **current** › next…" trail (windowed to fit its width).
    /// `None` when the topology has no regions or the card isn't in one.
    pub fn region_path(&self, card: &str) -> Option<(Vec<&str>, usize)> {
        let current = self
            .regions
            .iter()
            .position(|r| r.cards.iter().any(|c| c == card))?;
        let names = self.regions.iter().map(|r| r.name.as_str()).collect();
        Some((names, current))
    }

    /// Whether this topology belongs to a deck whose token is in `deck_tokens` —
    /// the replacement for the old any-card-overlap check. A shared cache holds
    /// several decks' topologies; this scopes to just the ones this deck (or, for
    /// a workspace screen, this set of member decks) owns.
    pub fn belongs_to(&self, deck_tokens: &HashSet<String>) -> bool {
        !self.deck_token.is_empty() && deck_tokens.contains(&self.deck_token)
    }

    /// The card ids of the region named `name` (matched case-insensitively), for
    /// focusing a session on one area. `None` when no region matches.
    pub fn region_cards(&self, name: &str) -> Option<&[String]> {
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
    rank: HashMap<String, usize>,
}

impl TopologyOrder {
    /// Builds the lookup from a topology's walk (card ids in walk order).
    pub fn from_walk(walk: &[String]) -> Self {
        Self {
            rank: walk
                .iter()
                .enumerate()
                .map(|(i, id)| (id.clone(), i))
                .collect(),
        }
    }

    /// The card's position in the walk, or `None` if it isn't on the walk.
    pub fn rank_of(&self, card_id: &str) -> Option<usize> {
        self.rank.get(card_id).copied()
    }
}

/// On-disk representation of the cache.
#[derive(Serialize, Deserialize)]
struct AugmentFile {
    /// Format version; defaults to the oldest for a file written before the
    /// field existed.
    #[serde(default = "default_version")]
    version: u32,
    /// Augmentations keyed by the card's identity token (its `Card::id`).
    cards: HashMap<String, Augmentation>,
    /// The deck-level topologies, one per organizing principle. Additive and
    /// defaulted so caches without it keep loading.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    topologies: Vec<Topology>,
}

/// A best-effort, id-keyed cache of AI augmentations for cards, plus optional
/// deck-level [`Topology`] objects.
pub struct AugmentCache {
    path: PathBuf,
    cards: HashMap<String, Augmentation>,
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

    /// The cached distractors for a card, or `None` when absent or empty — so
    /// the caller can fall back to offline sampling with a single check.
    pub fn distractors(&self, card_id: &str) -> Option<&[String]> {
        self.cards
            .get(card_id)
            .map(|aug| aug.distractors.as_slice())
            .filter(|d| !d.is_empty())
    }

    /// Whether an augmentation is already cached for a card. Used to warm only
    /// the cards that still need it.
    pub fn contains(&self, card_id: &str) -> bool {
        self.cards.contains_key(card_id)
    }

    /// Stores the distractors for a card, replacing any previous ones. Does not
    /// save.
    pub fn set_distractors(&mut self, card_id: &str, distractors: Vec<String>) {
        self.cards
            .entry(card_id.to_string())
            .or_default()
            .distractors = distractors;
    }

    /// The cached note for a card, if any.
    pub fn note(&self, card_id: &str) -> Option<&str> {
        self.cards.get(card_id).and_then(|aug| aug.note.as_deref())
    }

    /// Stores a generated note for a card, replacing any previous one. Does not
    /// save.
    pub fn set_note(&mut self, card_id: &str, note: String) {
        self.cards.entry(card_id.to_string()).or_default().note = Some(note);
    }

    /// The cached presentation reshape for a card, if any.
    pub fn format(&self, card_id: &str) -> Option<&Format> {
        self.cards.get(card_id).and_then(|aug| aug.format.as_ref())
    }

    /// Caches a presentation reshape for a card.
    pub fn set_format(&mut self, card_id: &str, format: Format) {
        self.cards.entry(card_id.to_string()).or_default().format = Some(format);
    }

    /// The cached question variants for a card (a pool to rotate through), or
    /// `None` when absent or empty.
    pub fn variants(&self, card_id: &str) -> Option<&[String]> {
        self.cards
            .get(card_id)
            .map(|aug| aug.variants.as_slice())
            .filter(|v| !v.is_empty())
    }

    /// Picks a question phrasing for a card from the pool of the authored
    /// `original` plus the cached variants, rotating by `seed` (a plain modulo).
    /// The original sits at index 0, so it stays in the rotation. `None` when no
    /// variants are cached — the caller then keeps the original front unchanged.
    pub fn pick_front(&self, card_id: &str, original: &str, seed: u64) -> Option<String> {
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
    pub fn set_variants(&mut self, card_id: &str, variants: Vec<String>) {
        self.cards.entry(card_id.to_string()).or_default().variants = variants;
    }

    /// The cached key points for a card (the Explain-mode checklist rubric), or
    /// `None` when absent or empty — so the caller can fall back to the plain
    /// self-graded reveal with one check.
    pub fn keypoints(&self, card_id: &str) -> Option<&[String]> {
        self.cards
            .get(card_id)
            .map(|aug| aug.keypoints.as_slice())
            .filter(|k| !k.is_empty())
    }

    /// Stores the key points for a card, replacing any previous ones. Does not
    /// save.
    pub fn set_keypoints(&mut self, card_id: &str, keypoints: Vec<String>) {
        self.cards.entry(card_id.to_string()).or_default().keypoints = keypoints;
    }

    /// All cached deck-level topologies, one per organizing principle.
    pub fn topologies(&self) -> &[Topology] {
        &self.topologies
    }

    /// The cached topologies belonging to a deck whose token is in `deck_tokens`.
    /// A cache file is shared by every deck that shares a store (e.g. all loose
    /// decks under the global store), so this filters out other decks' topologies
    /// by owner token — see [`Topology::belongs_to`]. For a plain deck the set is
    /// its one token; for a workspace screen it is every member's.
    pub fn topologies_for(&self, deck_tokens: &HashSet<String>) -> Vec<&Topology> {
        self.topologies
            .iter()
            .filter(|t| t.belongs_to(deck_tokens))
            .collect()
    }

    /// Whether any cached topology belongs to a deck whose token is in
    /// `deck_tokens` — i.e. the picker's focus drawer would open for it. Cheaper
    /// than [`topologies_for`](Self::topologies_for) when only presence matters
    /// (no allocation).
    pub fn has_topology_for(&self, deck_tokens: &HashSet<String>) -> bool {
        self.topologies.iter().any(|t| t.belongs_to(deck_tokens))
    }

    /// The cached topology with the given [`name`](Topology::name), if any — how
    /// the scheduler selects which one to walk.
    pub fn topology(&self, name: &str) -> Option<&Topology> {
        self.topologies.iter().find(|t| t.name == name)
    }

    /// Stores a topology. It replaces an existing one with the same
    /// [`name`](Topology::name) **and the same owner deck**
    /// ([`deck_token`](Topology::deck_token)) — re-running a principle refreshes
    /// it — and otherwise appends. The owner check matters because a cache can be
    /// shared by several decks (one store): a like-named topology from another
    /// deck (e.g. both defaulting to `auto`) must not be clobbered. Does not
    /// save.
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

    /// MOVE a cloze card's hole-keyed cache entries to follow the realignment
    /// cascade (spec §3.4, the D2 cross-family rule: MOVE, never invalidate — a
    /// displaced hole must never inherit another word's distractors, and a live
    /// hole must never lose its cached choices to a re-index). Per the
    /// [`CascadeOutcome`], each matched hole's `token-<old>` augmentation and any
    /// topology reference move to `token-<new>`; orphaned holes' entries drop;
    /// fresh holes have none until augmented. Returns whether anything changed
    /// (so the caller can skip an unnecessary save). Does not save.
    pub fn remap_holes(&mut self, token: &str, outcome: &crate::store::CascadeOutcome) -> bool {
        // An identity remap with nothing orphaned moves no id (a fresh hole
        // appended at the end adds no old key to rewrite).
        let identity = outcome.remap.iter().all(|(from, to)| from == to);
        if identity && outcome.orphaned.is_empty() {
            return false;
        }
        let moves: HashMap<u32, u32> = outcome.remap.iter().copied().collect();

        // Per-card augmentations: pull every stored hole's entry (matched or
        // orphaned), then re-insert only the matched ones at their new index —
        // a fresh rebuild, so a fresh hole's key is left empty and an orphaned
        // hole's entry drops.
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

        // Deck-level topology references (walk order, region membership, edges):
        // a hole id in any of them is rewritten to its new index, or dropped
        // when its hole orphaned.
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
    pub fn summarize(&self, cards: &[Card], deck_tokens: &HashSet<String>) -> CoverageSummary {
        let coverage = |eligible: &[&Card], covered: &dyn Fn(&str) -> bool| Coverage {
            covered: eligible
                .iter()
                .filter(|c| c.id().is_some_and(|id| covered(&id)))
                .count(),
            eligible: eligible.len(),
        };
        let all: Vec<&Card> = cards.iter().collect();
        let plain: Vec<&Card> = cards.iter().filter(|c| c.hash_lines.is_none()).collect();
        CoverageSummary {
            choices: coverage(&all, &|id| self.distractors(id).is_some()),
            notes: coverage(&all, &|id| self.note(id).is_some()),
            questions: coverage(&plain, &|id| self.variants(id).is_some()),
            keypoints: coverage(&all, &|id| self.keypoints(id).is_some()),
            format: coverage(&plain, &|id| self.format(id).is_some()),
            topologies: self
                .topologies_for(deck_tokens)
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
        covered: impl Fn(&str) -> bool,
    ) -> Vec<WarmItem> {
        cards
            .iter()
            .filter(|c| eligible(c) && c.id().is_some_and(|id| !covered(&id)))
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
    pub fn clear_distractors(&mut self, deck_ids: &HashSet<String>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.distractors.clear();
            }
        }
        self.prune_empty(deck_ids);
    }

    /// Removes this deck's cached notes (see [`clear_distractors`](Self::clear_distractors)).
    pub fn clear_notes(&mut self, deck_ids: &HashSet<String>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.note = None;
            }
        }
        self.prune_empty(deck_ids);
    }

    /// Removes this deck's cached question variants (see
    /// [`clear_distractors`](Self::clear_distractors)).
    pub fn clear_variants(&mut self, deck_ids: &HashSet<String>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.variants.clear();
            }
        }
        self.prune_empty(deck_ids);
    }

    /// Removes this deck's cached key points (see [`clear_distractors`](Self::clear_distractors)).
    pub fn clear_keypoints(&mut self, deck_ids: &HashSet<String>) {
        for id in deck_ids {
            if let Some(aug) = self.cards.get_mut(id) {
                aug.keypoints.clear();
            }
        }
        self.prune_empty(deck_ids);
    }

    /// Removes cached reshapes for this deck, then prunes empty entries.
    pub fn clear_format(&mut self, deck_ids: &HashSet<String>) {
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
        let Some(fmt) = card.id().and_then(|id| self.format(&id)) else {
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
    fn prune_empty(&mut self, deck_ids: &HashSet<String>) {
        for id in deck_ids {
            if self.cards.get(id).is_some_and(Augmentation::is_empty) {
                self.cards.remove(id);
            }
        }
    }

    /// Removes the named topology if it belongs to a deck in `deck_tokens` (its
    /// name matches **and** it [`belongs_to`](Topology::belongs_to) one of those
    /// decks, so a like-named topology from another deck on a shared store is
    /// left alone). Returns whether one was removed. Does not save.
    pub fn remove_topology(&mut self, name: &str, deck_tokens: &HashSet<String>) -> bool {
        let before = self.topologies.len();
        self.topologies
            .retain(|t| !(t.name == name && t.belongs_to(deck_tokens)));
        self.topologies.len() != before
    }

    /// Removes every augmentation this deck owns — all per-card fields for
    /// `deck_ids` and all topologies belonging to a deck in `deck_tokens` (the
    /// "remove all" action). Other decks sharing the cache are untouched. Does
    /// not save.
    pub fn clear_all(&mut self, deck_ids: &HashSet<String>, deck_tokens: &HashSet<String>) {
        for id in deck_ids {
            self.cards.remove(id);
        }
        self.topologies.retain(|t| !t.belongs_to(deck_tokens));
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
    cards: HashMap<String, Augmentation>,
    topologies: Vec<Topology>,
}

/// Loads the cache, returning `None` on any problem (missing/corrupt/newer file)
/// so [`AugmentCache::open`] can fall back to empty. A cache written before the
/// token flip (numeric card keys and `u64` topology ids) fails to deserialize
/// into the string-keyed shape, so it lenient-fails here and is regenerated —
/// the intended pre-1.0 break.
fn load(path: &Path) -> Option<Loaded> {
    let text = std::fs::read_to_string(path).ok()?;
    let file: AugmentFile = serde_json::from_str(&text).ok()?;
    // A cache from a newer alix may hold a shape we'd mangle — ignore it and
    // regenerate rather than risk serving wrong options.
    if file.version > CURRENT_VERSION {
        return None;
    }
    // Card keys are identity tokens (strings): they pass through verbatim.
    Some(Loaded {
        cards: file.cards,
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
    /// The card's identity token (the cache key). Warm items are built over
    /// stamped cards (augment open excludes unstamped ones), so an empty id here
    /// would mean an unstamped card slipped the boundary — impossible in
    /// practice.
    pub id: String,
    /// The question shown to the learner (the card front).
    pub question: String,
    /// The correct answer the augmentation must respect.
    pub answer: String,
    /// The card's deck note, if any — used by the format target to re-render it.
    pub note: Option<String>,
}

impl WarmItem {
    /// Builds the generation input for a card: its identity token, its front,
    /// its joined back, and its deck note.
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
    fn augment_entries_move_with_their_hole() {
        // Cross-family MOVE (spec §3.4, D2, user-ruled): choices are cached for
        // hole 0; a new hole is inserted before it, so the old hole is now hole
        // 1. Its distractors must MOVE to `token-1` (never invalidate, never
        // stay under `token-0` where a different word would inherit them).
        use crate::store::CascadeOutcome;
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        cache.set_distractors("tok-0", vec!["wrong x".into(), "wrong y".into()]);
        cache.set_note("tok-0", "a note about the old hole 0".into());

        // Insert a hole at the front: old hole 0 → file hole 1; file hole 0 is
        // fresh (has no augmentation yet).
        let outcome = CascadeOutcome {
            remap: vec![(0, 1)],
            orphaned: vec![],
            fresh: vec![0],
        };
        assert!(cache.remap_holes("tok", &outcome));

        // The choices + note now live under token-1…
        assert_eq!(
            Some(["wrong x".to_string(), "wrong y".to_string()].as_slice()),
            cache.distractors("tok-1")
        );
        assert_eq!(Some("a note about the old hole 0"), cache.note("tok-1"));
        // …and the fresh hole 0 has none (a displaced hole never inherits
        // another word's distractors).
        assert!(cache.distractors("tok-0").is_none());
        assert!(cache.note("tok-0").is_none());
    }

    #[test]
    fn an_orphaned_holes_augmentation_is_dropped_not_inherited() {
        // A hole whose word vanished orphans: its cached entry is dropped, never
        // left under a live key for a new word to inherit.
        use crate::store::CascadeOutcome;
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        cache.set_distractors("tok-0", vec!["a".into()]);
        let outcome = CascadeOutcome {
            remap: vec![],
            orphaned: vec![0],
            fresh: vec![0],
        };
        assert!(cache.remap_holes("tok", &outcome));
        assert!(cache.distractors("tok-0").is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");

        let mut cache = AugmentCache::open(&path);
        cache.set_distractors("c42", vec!["wrong a".into(), "wrong b".into()]);
        cache.save().unwrap();

        let reloaded = AugmentCache::open(&path);
        assert_eq!(1, reloaded.len());
        assert_eq!(
            Some(["wrong a".to_string(), "wrong b".to_string()].as_slice()),
            reloaded.distractors("c42")
        );
    }

    #[test]
    fn distractors_is_none_when_absent_or_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        assert_eq!(None, cache.distractors("c1")); // absent
        cache.set_distractors("c1", Vec::new());
        assert_eq!(None, cache.distractors("c1")); // present but empty
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
        // Card keys are identity tokens now, so any string key passes through —
        // an odd-charset key is kept (it becomes doctor material) rather than
        // silently dropped.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        std::fs::write(
            &path,
            r#"{"version":1,"cards":{"not-a-token":{"distractors":["x"]},"q7":{"distractors":["y"]}}}"#,
        )
        .unwrap();
        let cache = AugmentCache::open(&path);
        assert_eq!(2, cache.len());
        assert_eq!(Some(["y".to_string()].as_slice()), cache.distractors("q7"));
        assert_eq!(
            Some(["x".to_string()].as_slice()),
            cache.distractors("not-a-token")
        );
    }

    #[test]
    fn file_without_version_field_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        std::fs::write(&path, r#"{"cards":{"c3":{"distractors":["z"]}}}"#).unwrap();
        let cache = AugmentCache::open(&path);
        assert_eq!(Some(["z".to_string()].as_slice()), cache.distractors("c3"));
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
        cache.set_distractors("c1", vec!["old".into()]);
        cache.set_distractors("c1", vec!["new a".into(), "new b".into()]);
        assert_eq!(
            Some(["new a".to_string(), "new b".to_string()].as_slice()),
            cache.distractors("c1")
        );
    }

    #[test]
    fn note_roundtrips_through_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        let mut cache = AugmentCache::open(&path);
        cache.set_note("c7", "a memorable fact".into());
        cache.save().unwrap();
        let reloaded = AugmentCache::open(&path);
        assert_eq!(Some("a memorable fact"), reloaded.note("c7"));
        assert_eq!(None, reloaded.note("c8"));
    }

    #[test]
    fn variants_roundtrip_and_pick() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        let mut cache = AugmentCache::open(&path);
        cache.set_variants("c5", vec!["one".into(), "two".into(), "three".into()]);
        cache.save().unwrap();
        let reloaded = AugmentCache::open(&path);
        assert_eq!(3, reloaded.variants("c5").unwrap().len());
        // pool = [original] + 3 variants = 4; idx = seed % 4, original at 0.
        assert_eq!(
            Some("ORIG".to_string()),
            reloaded.pick_front("c5", "ORIG", 0)
        );
        assert_eq!(
            Some("one".to_string()),
            reloaded.pick_front("c5", "ORIG", 1)
        );
        assert_eq!(
            Some("ORIG".to_string()),
            reloaded.pick_front("c5", "ORIG", 4)
        ); // 4 % 4 == 0
        assert_eq!(None, reloaded.pick_front("c6", "ORIG", 0)); // no variants
    }

    #[test]
    fn keypoints_roundtrip_through_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("augment.json");
        let mut cache = AugmentCache::open(&path);
        cache.set_keypoints("c9", vec!["claim a".into(), "claim b".into()]);
        cache.save().unwrap();
        let reloaded = AugmentCache::open(&path);
        assert_eq!(
            Some(["claim a".to_string(), "claim b".to_string()].as_slice()),
            reloaded.keypoints("c9")
        );
        assert_eq!(None, reloaded.keypoints("c10")); // none cached
    }

    /// A HashSet of deck tokens, the scoping key for the topology methods.
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

    /// The region cards of a topology as `&str`s, for comparing against literals.
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

        // Re-running the same principle over the same deck (same owner token)
        // refreshes it in place, not appends.
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
        // Two decks sharing a store both default to the name `auto`; their owner
        // tokens differ, so the second must NOT clobber the first.
        let mut cache = AugmentCache::open(std::path::Path::new("unused.json"));
        cache.add_topology(topology("auto", "dA", &["c1", "c2", "c3"])); // deck A
        cache.add_topology(topology("auto", "dB", &["c10", "c20", "c30"])); // deck B
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
        // Card `c1` used to live in deck A (its topology walks it) and later moved
        // to deck B, keeping its token. Under the old any-card-overlap check, deck
        // B (which now contains `c1`) would wrongly "own" deck A's topology. Owner
        // tokens fix it: the topology stays bound to deck A, never leaks to B.
        let mut cache = AugmentCache::open(std::path::Path::new("unused.json"));
        cache.add_topology(topology("auto", "dA", &["c1", "c2"]));

        // Deck B's screen (its own token `dB`) sees no topology, even though it
        // now contains card `c1` from deck A's walk.
        assert!(!cache.has_topology_for(&tokens(&["dB"])));
        assert!(cache.topologies_for(&tokens(&["dB"])).is_empty());
        // Deck A still owns it.
        assert_eq!(1, cache.topologies_for(&tokens(&["dA"])).len());
    }

    #[test]
    fn topologies_for_keeps_only_the_decks_own() {
        // One cache shared by two decks (they share a store): each topology is
        // owner-tagged, so `topologies_for` returns only the asked-for deck's —
        // no cross-deck leak.
        let mut cache = AugmentCache::open(std::path::Path::new("unused.json"));
        cache.add_topology(topology("architecture", "dA", &["c1", "c2", "c3"]));
        cache.add_topology(topology("capitals", "dB", &["c10", "c20", "c30"]));

        let mine = cache.topologies_for(&tokens(&["dA"]));
        assert_eq!(1, mine.len());
        assert_eq!("architecture", mine[0].name);

        // A deck sharing the store but with a different token sees no topology.
        assert!(cache.topologies_for(&tokens(&["dZ"])).is_empty());
    }

    #[test]
    fn has_topology_for_reports_presence_without_cross_deck_leak() {
        let mut cache = AugmentCache::open(std::path::Path::new("unused.json"));
        cache.add_topology(topology("architecture", "dA", &["c1", "c2", "c3"]));

        assert!(cache.has_topology_for(&tokens(&["dA"])));
        // A deck sharing the store but with a different token has no drawer.
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
        assert_eq!(1, current); // card c3 lives in "Session"
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
        assert!(t.region_path("c99").is_none()); // card in no region
        assert!(topo_regions(vec![]).region_path("c1").is_none()); // no regions at all
    }

    #[test]
    fn topology_order_from_walk_ranks_present_and_misses_absent() {
        let walk = ["c10".to_string(), "c20".to_string(), "c30".to_string()];
        let order = TopologyOrder::from_walk(&walk);
        assert_eq!(Some(0), order.rank_of("c10"));
        assert_eq!(Some(2), order.rank_of("c30"));
        assert_eq!(None, order.rank_of("c99"));
    }

    // ── Coverage / gaps / removal (the web Augment screen's lib backing) ──

    /// A stamped plain card whose token derives from `back`, so distinct
    /// backs give distinct identities for the cache keys below.
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

    /// A stamped card's id, for the coverage tests (all cards here are stamped).
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
        cache.set_distractors(&cid(&cards[0]), vec!["x".into()]);
        cache.set_distractors(&cid(&cards[1]), vec!["y".into()]);
        cache.set_note(&cid(&cards[0]), "n".into());
        cache.set_variants(&cid(&cards[0]), vec!["v".into()]);
        cache.set_keypoints(&cid(&cards[2]), vec!["k1".into(), "k2".into()]);
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
        cache.set_distractors(&cid(&cards[0]), vec!["x".into()]);

        let miss: Vec<String> = cache
            .missing_choices(&cards)
            .iter()
            .map(|w| w.id.clone())
            .collect();
        assert_eq!(miss, [cid(&cards[1]), cid(&cards[2])]); // a covered; b + z still need it

        // Questions exclude cloze cards entirely, covered or not.
        let mq: Vec<String> = cache
            .missing_questions(&cards)
            .iter()
            .map(|w| w.id.clone())
            .collect();
        assert_eq!(mq, [cid(&cards[0]), cid(&cards[1])]);
    }

    #[test]
    fn clear_distractors_is_deck_scoped_and_prunes_empty_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        let mine = plain_card("a");
        let other = plain_card("other-deck-card");
        cache.set_distractors(&cid(&mine), vec!["x".into()]);
        cache.set_distractors(&cid(&other), vec!["y".into()]);

        let deck_ids: HashSet<String> = [cid(&mine)].into_iter().collect();
        cache.clear_distractors(&deck_ids);

        assert_eq!(None, cache.distractors(&cid(&mine)));
        assert!(!cache.contains(&cid(&mine))); // held nothing else → pruned
        // The other deck sharing this cache is untouched.
        assert_eq!(
            Some(["y".to_string()].as_slice()),
            cache.distractors(&cid(&other))
        );
    }

    #[test]
    fn clear_notes_keeps_other_fields_and_does_not_prune() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        let c = plain_card("a");
        cache.set_note(&cid(&c), "n".into());
        cache.set_distractors(&cid(&c), vec!["x".into()]);

        let deck_ids: HashSet<String> = [cid(&c)].into_iter().collect();
        cache.clear_notes(&deck_ids);

        assert_eq!(None, cache.note(&cid(&c)));
        assert_eq!(
            Some(["x".to_string()].as_slice()),
            cache.distractors(&cid(&c))
        );
        assert!(cache.contains(&cid(&c))); // still has distractors → not pruned
    }

    #[test]
    fn remove_topology_is_name_and_deck_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        let mine = plain_card("a");
        let other = plain_card("other");
        cache.add_topology(topo_over("auto", "dA", &cid(&mine)));
        cache.add_topology(topo_over("auto", "dB", &cid(&other))); // same name, other deck

        assert!(cache.remove_topology("auto", &tokens(&["dA"])));
        assert_eq!(1, cache.topologies().len());
        assert_eq!(1, cache.topologies_for(&tokens(&["dB"])).len()); // the other deck's survives
        assert!(!cache.remove_topology("nope", &tokens(&["dA"]))); // no match → false
    }

    #[test]
    fn clear_all_removes_only_this_decks_augmentations() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AugmentCache::open(dir.path().join("augment.json"));
        let mine = plain_card("a");
        let other = plain_card("other");
        cache.set_distractors(&cid(&mine), vec!["x".into()]);
        cache.set_note(&cid(&mine), "n".into());
        cache.add_topology(topo_over("auto", "dA", &cid(&mine)));
        cache.set_distractors(&cid(&other), vec!["y".into()]);
        cache.add_topology(topo_over("auto", "dB", &cid(&other)));

        let deck_ids: HashSet<String> = [cid(&mine)].into_iter().collect();
        cache.clear_all(&deck_ids, &tokens(&["dA"]));

        assert!(!cache.contains(&cid(&mine)));
        assert!(cache.topologies_for(&tokens(&["dA"])).is_empty());
        // The other deck is intact.
        assert_eq!(
            Some(["y".to_string()].as_slice()),
            cache.distractors(&cid(&other))
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
        );
        cache.apply_format(&mut card);
        assert_eq!(card.front, "Name the parts");
        assert_eq!(card.back_for_display(), ["A", "B", "C"]);
        // The suggested `line` mode is applied as the `line` reveal-method.
        assert_eq!(card.reveal, Some(Reveal::Line));
        assert_eq!(cid(&card), id); // identity preserved
    }

    #[test]
    fn apply_format_respects_an_explicit_reveal() {
        use std::sync::Arc;
        let mut card = Card::plain(Arc::from("d.md"), "f".into(), vec!["a".into()], None, 1);
        card.token = Some(Arc::from("qfmt2"));
        card.reveal = Some(Reveal::Flip); // user's explicit choice
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
        );
        cache.apply_format(&mut card);
        assert_eq!(card.reveal, Some(Reveal::Flip)); // suggestion does not override
    }
}
