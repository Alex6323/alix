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

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ask,
    config::{AiConfig, AskConfig},
};

/// The on-disk cache-format version. Bumped only if the persisted shape changes
/// incompatibly; because the cache is regenerable, a newer version is ignored
/// (an empty cache is returned) rather than refused.
const CURRENT_VERSION: u32 = 1;

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

// ── Generation ───────────────────────────────────────────────────────────────
//
// Distractors come from one batched, tool-free Claude call over the cards that
// still need them, mirroring the exam's generate/grade shape: a synchronous core
// ([`generate`]) the interactive frontends run on a thread via [`spawn`]. The
// call is pure text transformation — no web or file tools — so its allowlist is
// cleared like exam remediation.

/// One card to generate distractors for.
#[derive(Clone, Debug)]
pub struct WarmItem {
    /// The card's identity hash (the cache key).
    pub id: u64,
    /// The question shown to the learner (the card front).
    pub question: String,
    /// The correct answer the distractors must be wrong about and distinct from.
    pub answer: String,
}

/// Generates up to `count` distractors per card in `items` with one batched,
/// tool-free call, optionally steered by `guidance` (the `--with` text). Returns
/// a map from card id to its validated distractors; cards the model produced
/// nothing usable for are omitted, so review falls back to offline sampling.
pub fn generate(
    items: &[WarmItem],
    count: usize,
    guidance: Option<&str>,
    ask_cfg: &AskConfig,
) -> Result<HashMap<u64, Vec<String>>> {
    if items.is_empty() {
        return Ok(HashMap::new());
    }
    let prompt = distractors_prompt(items, count, guidance);
    let raw = ask::run(&tool_free(ask_cfg), &prompt, &[])?;
    let parsed: HashMap<String, Vec<String>> =
        parse_json(&raw).context("parsing the generated distractors")?;

    let mut out = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        let Some(raw_options) = parsed.get(&index.to_string()) else {
            continue;
        };
        let cleaned = clean_distractors(raw_options, &item.answer, count);
        if !cleaned.is_empty() {
            out.insert(item.id, cleaned);
        }
    }
    Ok(out)
}

/// Generates one short note (trivia, context, or a mnemonic) per card in `items`,
/// optionally steered by `guidance`. Returns card id → note, omitting any the
/// model left blank.
pub fn generate_notes(
    items: &[WarmItem],
    guidance: Option<&str>,
    ask_cfg: &AskConfig,
) -> Result<HashMap<u64, String>> {
    if items.is_empty() {
        return Ok(HashMap::new());
    }
    let prompt = notes_prompt(items, guidance);
    let raw = ask::run(&tool_free(ask_cfg), &prompt, &[])?;
    let parsed: HashMap<String, String> =
        parse_json(&raw).context("parsing the generated notes")?;

    let mut out = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        if let Some(note) = parsed.get(&index.to_string()) {
            let note = note.trim();
            if !note.is_empty() {
                out.insert(item.id, note.to_string());
            }
        }
    }
    Ok(out)
}

/// Generates up to `count` reworded phrasings of each card's question, steered
/// by `guidance`, each keeping the **exact same answer**. Returns card id → a
/// pool of variants (rotated at review time); cards the model produced nothing
/// usable for are omitted.
pub fn generate_variants(
    items: &[WarmItem],
    count: usize,
    guidance: Option<&str>,
    ask_cfg: &AskConfig,
) -> Result<HashMap<u64, Vec<String>>> {
    if items.is_empty() {
        return Ok(HashMap::new());
    }
    let prompt = variants_prompt(items, count, guidance);
    let raw = ask::run(&tool_free(ask_cfg), &prompt, &[])?;
    let parsed: HashMap<String, Vec<String>> =
        parse_json(&raw).context("parsing the generated question variants")?;

    let mut out = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        let Some(raw_variants) = parsed.get(&index.to_string()) else {
            continue;
        };
        let cleaned = clean_variants(raw_variants, &item.question, count);
        if !cleaned.is_empty() {
            out.insert(item.id, cleaned);
        }
    }
    Ok(out)
}

/// The model's raw topology before card indices are mapped back to identity
/// hashes.
#[derive(Deserialize)]
struct RawTopology {
    #[serde(default)]
    principle: String,
    #[serde(default)]
    edges: Vec<RawEdge>,
    #[serde(default)]
    walk: Vec<usize>,
    #[serde(default)]
    regions: Vec<RawRegion>,
}

/// A raw edge addressed by the cards' positions in the prompt listing.
#[derive(Deserialize)]
struct RawEdge {
    from: usize,
    to: usize,
    #[serde(default)]
    label: String,
}

/// A raw region: a name plus the cards' positions in the prompt listing.
#[derive(Deserialize)]
struct RawRegion {
    #[serde(default)]
    name: String,
    #[serde(default)]
    cards: Vec<usize>,
}

/// Derives a single deck-level [`Topology`] over `items` in one batched,
/// tool-free call, steered by `guidance` (the favored organizing principle).
/// Indices the model returns are mapped back to card identity hashes; any out of
/// range are dropped rather than failing the whole call.
pub fn generate_topology(
    items: &[WarmItem],
    guidance: Option<&str>,
    ask_cfg: &AskConfig,
) -> Result<Topology> {
    if items.is_empty() {
        return Ok(Topology::default());
    }
    let prompt = topology_prompt(items, guidance);
    let raw = ask::run(&tool_free(ask_cfg), &prompt, &[])?;
    let parsed: RawTopology = parse_json(&raw).context("parsing the generated topology")?;
    let mut topology = to_topology(parsed, items);
    topology.name = guidance
        .map(|g| g.trim())
        .filter(|g| !g.is_empty())
        .unwrap_or("auto")
        .to_string();
    Ok(topology)
}

/// Maps a [`RawTopology`]'s card indices back to identity hashes, dropping any
/// index outside `items` and any card repeated in the walk.
fn to_topology(raw: RawTopology, items: &[WarmItem]) -> Topology {
    let id_of = |idx: usize| items.get(idx).map(|it| it.id);
    let edges = raw
        .edges
        .into_iter()
        .filter_map(|e| {
            Some(TopologyEdge {
                from: id_of(e.from)?,
                to: id_of(e.to)?,
                label: e.label.trim().to_string(),
            })
        })
        .collect();
    let mut seen = HashSet::new();
    let walk = raw
        .walk
        .into_iter()
        .filter_map(id_of)
        .filter(|id| seen.insert(*id))
        .collect();
    let regions = raw
        .regions
        .into_iter()
        .map(|r| TopologyRegion {
            name: r.name.trim().to_string(),
            cards: r.cards.into_iter().filter_map(id_of).collect(),
        })
        .filter(|r| !r.name.is_empty() && !r.cards.is_empty())
        .collect();
    Topology {
        // Filled in by the caller from the `--with` guidance.
        name: String::new(),
        principle: raw.principle.trim().to_string(),
        edges,
        walk,
        regions,
    }
}

/// Builds the topology prompt: list the cards, ask for an organizing principle, a
/// labeled edge set, and a walk that visits every card so consecutive ones relate.
fn topology_prompt(items: &[WarmItem], guidance: Option<&str>) -> String {
    let mut s = String::from(
        "You are organizing a set of flashcards into a TOPOLOGY: a graph of how \
         the facts relate, so a learner can be quizzed in a connected order \
         instead of at random. The aim is that each card feels like a natural \
         follow-up to the one before it (\"same module\", \"also in Europe\", \
         \"this type is built from that one\").\n\n\
         Decide an organizing principle, then give:\n\
         - edges: directed links `from` → `to` meaning \"after the `from` card, \
         the `to` card is a sensible next step\", each with a short `label` \
         saying why they relate;\n\
         - walk: an order to visit EVERY card (by index) such that consecutive \
         cards are related — your default path through the graph;\n\
         - regions: 3–7 coarse named groups (stages or themes) covering the \
         cards, listed in the order the walk passes through them. Each region \
         has a short place-NAME (one or two words, not a sentence) and the \
         indices of its cards; every card belongs to exactly one region. The \
         name must orient WITHOUT giving away any card's answer — name the area, \
         never the fact (\"Persistence\", not \"saves to progress.json\").\n\
         Use the card indices below. Relate cards by their meaning, not their \
         wording.\n",
    );
    if let Some(g) = guidance {
        s.push_str(&format!("\nFavored organizing principle: {}\n", g.trim()));
    }
    s.push_str("\nCards (index. question — answer):\n");
    for (i, item) in items.iter().enumerate() {
        s.push_str(&format!(
            "{i}. {} — {}\n",
            one_line(&item.question),
            one_line(&item.answer)
        ));
    }
    s.push_str(
        "\nOutput ONLY JSON in exactly this shape, no prose, no code fences:\n\
         {\"principle\": \"...\", \
         \"edges\": [{\"from\": 0, \"to\": 1, \"label\": \"...\"}], \
         \"walk\": [0, 1, ...], \
         \"regions\": [{\"name\": \"...\", \"cards\": [0, 1]}]}\n",
    );
    s
}

/// A copy of `ask` with the tool allowlist cleared — generation is a pure text
/// call that needs no web or file access (like exam remediation).
fn tool_free(ask: &AskConfig) -> AskConfig {
    let mut cfg = ask.clone();
    cfg.allowed_tools.clear();
    cfg
}

/// Builds the [`AskConfig`] for a generation call from the base `[ask]` config
/// plus the `[ai]` overrides: the AI model (falling back to `[ask]`'s), the AI
/// timeout, and a cleared tool allowlist (generation is a pure text call that
/// needs no web or file access).
pub fn run_config(ai: &AiConfig, ask: &AskConfig) -> AskConfig {
    let mut cfg = ask.clone();
    if ai.model.is_some() {
        cfg.model = ai.model.clone();
    }
    cfg.timeout_secs = ai.timeout_secs;
    cfg.allowed_tools.clear();
    cfg
}

/// Builds the batched distractor prompt: each card as `index. question —
/// answer`, then a strict JSON output shape keyed by that index.
fn distractors_prompt(items: &[WarmItem], count: usize, guidance: Option<&str>) -> String {
    let mut s = format!(
        "You are writing distractors — plausible but incorrect options — for \
         multiple-choice flashcards.\n\n\
         For each card, give exactly {count} wrong answers that:\n\
         - are tempting to someone who only half-knows the material,\n\
         - match the form and length of the correct answer (a year competes \
         with years, a command with commands),\n\
         - are clearly incorrect — never a synonym or restatement of the correct \
         answer,\n\
         - are distinct from each other and from the correct answer.\n"
    );
    if let Some(g) = guidance {
        s.push_str(&format!("\nExtra guidance: {}\n", g.trim()));
    }
    s.push_str("\nCards (index. question — correct answer):\n");
    for (i, item) in items.iter().enumerate() {
        s.push_str(&format!(
            "{i}. {} — {}\n",
            one_line(&item.question),
            one_line(&item.answer)
        ));
    }
    let slots = vec!["\"...\""; count].join(", ");
    s.push_str(&format!(
        "\nOutput ONLY JSON in exactly this shape, no prose, no code fences — \
         the key is the card index, the value its {count} distractors:\n\
         {{\"0\": [{slots}], ...}}\n"
    ));
    s
}

/// Builds the batched notes prompt: one short note per card, keyed by index.
fn notes_prompt(items: &[WarmItem], guidance: Option<&str>) -> String {
    let mut s = String::from(
        "You are adding a short note to each flashcard — one or two sentences of \
         memorable trivia, context, or a mnemonic that makes the answer easier to \
         recall. Keep each note tight and factual, and do not simply restate the \
         answer.\n",
    );
    if let Some(g) = guidance {
        s.push_str(&format!("\nExtra guidance: {}\n", g.trim()));
    }
    s.push_str("\nCards (index. question — answer):\n");
    for (i, item) in items.iter().enumerate() {
        s.push_str(&format!(
            "{i}. {} — {}\n",
            one_line(&item.question),
            one_line(&item.answer)
        ));
    }
    s.push_str(
        "\nOutput ONLY JSON, no prose, no code fences — the key is the card index, \
         the value its note as a single string:\n{\"0\": \"...\", ...}\n",
    );
    s
}

/// Builds the batched variants prompt: rephrase each question, keep the answer.
fn variants_prompt(items: &[WarmItem], count: usize, guidance: Option<&str>) -> String {
    let mut s = format!(
        "You are rephrasing flashcard questions. For each card, give {count} \
         different ways to ask the SAME question — reworded enough that a learner \
         must read and understand it, yet such that the EXACT same answer still \
         applies. Do not change what is being asked, do not add or drop \
         information, and never reveal or hint at the answer.\n"
    );
    if let Some(g) = guidance {
        s.push_str(&format!("\nExtra guidance: {}\n", g.trim()));
    }
    s.push_str("\nCards (index. question — the answer it must still have):\n");
    for (i, item) in items.iter().enumerate() {
        s.push_str(&format!(
            "{i}. {} — {}\n",
            one_line(&item.question),
            one_line(&item.answer)
        ));
    }
    let slots = vec!["\"...\""; count].join(", ");
    s.push_str(&format!(
        "\nOutput ONLY JSON in exactly this shape, no prose, no code fences — the \
         key is the card index, the value its {count} rephrasings:\n\
         {{\"0\": [{slots}], ...}}\n"
    ));
    s
}

/// Trims, drops empties, drops a rephrasing identical to the original question
/// (whitespace- and case-insensitively) or to one already kept, and caps at
/// `count`.
fn clean_variants(raw: &[String], original: &str, count: usize) -> Vec<String> {
    let norm = |s: &str| one_line(s).to_lowercase();
    let mut seen = HashSet::new();
    seen.insert(norm(original));
    let mut out = Vec::new();
    for variant in raw {
        let trimmed = variant.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(norm(trimmed)) {
            out.push(trimmed.to_string());
            if out.len() == count {
                break;
            }
        }
    }
    out
}

/// Collapses runs of whitespace (incl. newlines) so a multi-line front or back
/// stays on one line in the prompt listing.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Trims, drops empties, drops anything equal (case-insensitively) to the
/// correct answer or to an already-kept option, and caps the result at `count`.
fn clean_distractors(raw: &[String], answer: &str, count: usize) -> Vec<String> {
    let norm = |s: &str| s.trim().to_lowercase();
    let mut seen = HashSet::new();
    seen.insert(norm(answer));
    let mut out = Vec::new();
    for option in raw {
        let trimmed = option.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(norm(trimmed)) {
            out.push(trimmed.to_string());
            if out.len() == count {
                break;
            }
        }
    }
    out
}

/// The substring from the first `{` to the last `}`, so a JSON object survives
/// code fences or surrounding prose (mirrors the exam parser).
fn extract_json(raw: &str) -> &str {
    match (raw.find('{'), raw.rfind('}')) {
        (Some(a), Some(b)) if b > a => &raw[a..=b],
        _ => raw,
    }
}

/// Parses `raw` (possibly fenced / wrapped in prose) into `T`.
fn parse_json<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T> {
    let json = extract_json(raw);
    serde_json::from_str(json)
        .with_context(|| format!("the model did not return valid JSON:\n{json}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{AiConfig, AskConfig},
        testutil::{ask_config, exec_lock, fake_reply},
    };

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

    // ── generation ──

    fn item(id: u64, question: &str, answer: &str) -> WarmItem {
        WarmItem {
            id,
            question: question.into(),
            answer: answer.into(),
        }
    }

    #[test]
    fn generate_parses_and_maps_each_card_by_index() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(
            dir.path(),
            r#"{"0": ["w1","w2","w3"], "1": ["x1","x2","x3"]}"#,
        );
        let items = vec![
            item(10, "Capital of France?", "Paris"),
            item(20, "2+2?", "4"),
        ];
        let out = generate(&items, 3, None, &ask_config(&cli)).unwrap();
        assert_eq!(vec!["w1", "w2", "w3"], out[&10]);
        assert_eq!(vec!["x1", "x2", "x3"], out[&20]);
    }

    #[test]
    fn generate_drops_options_equal_to_the_answer_and_dedups() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // "paris"/"Paris" equal the answer (case-insensitively); "Lyon" repeats.
        let cli = fake_reply(
            dir.path(),
            r#"{"0": ["paris","Lyon","Lyon","Nice","Paris"]}"#,
        );
        let out = generate(
            &[item(1, "Capital of France?", "Paris")],
            3,
            None,
            &ask_config(&cli),
        )
        .unwrap();
        assert_eq!(vec!["Lyon", "Nice"], out[&1]);
    }

    #[test]
    fn generate_caps_at_count() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"0": ["a","b","c","d","e"]}"#);
        let out = generate(&[item(1, "q", "z")], 3, None, &ask_config(&cli)).unwrap();
        assert_eq!(3, out[&1].len());
    }

    #[test]
    fn generate_omits_a_card_with_no_usable_distractor() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // Card 0's options all equal the answer -> nothing usable -> omitted.
        let cli = fake_reply(dir.path(), r#"{"0": ["4","4"], "1": ["x1"]}"#);
        let out = generate(
            &[item(1, "2+2", "4"), item(2, "q", "y")],
            3,
            None,
            &ask_config(&cli),
        )
        .unwrap();
        assert!(!out.contains_key(&1));
        assert_eq!(vec!["x1"], out[&2]);
    }

    #[test]
    fn generate_malformed_json_is_an_error() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), "sorry, I can't do that");
        let err = generate(&[item(1, "q", "a")], 3, None, &ask_config(&cli)).unwrap_err();
        assert!(format!("{err:#}").contains("valid JSON"));
    }

    #[test]
    fn generate_with_no_items_makes_no_call() {
        // No real CLI: empty input must short-circuit to an empty map.
        let cfg = ask_config(Path::new("/nonexistent/claude"));
        assert!(generate(&[], 3, None, &cfg).unwrap().is_empty());
    }

    #[test]
    fn generate_notes_parses_each_card() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"0": "note a", "1": "note b"}"#);
        let items = vec![item(10, "q1", "a1"), item(20, "q2", "a2")];
        let out = generate_notes(&items, None, &ask_config(&cli)).unwrap();
        assert_eq!("note a", out[&10]);
        assert_eq!("note b", out[&20]);
    }

    #[test]
    fn generate_notes_omits_blank_notes() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"0": "   ", "1": "real note"}"#);
        let items = vec![item(1, "q", "a"), item(2, "q", "a")];
        let out = generate_notes(&items, None, &ask_config(&cli)).unwrap();
        assert!(!out.contains_key(&1));
        assert_eq!("real note", out[&2]);
    }

    #[test]
    fn generate_variants_drops_the_original_phrasing() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // The model echoes the original wording plus two genuine rewordings.
        let cli = fake_reply(
            dir.path(),
            r#"{"0": ["What year?", "In which year?", "Which year was it?"]}"#,
        );
        let out = generate_variants(&[item(1, "What year?", "1589")], 3, None, &ask_config(&cli))
            .unwrap();
        assert_eq!(vec!["In which year?", "Which year was it?"], out[&1]);
    }

    // ── topology ──

    #[test]
    fn generate_topology_parses_graph_and_walk() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(
            dir.path(),
            r#"{"principle":"by topic","edges":[{"from":0,"to":1,"label":"leads to"}],"walk":[0,1]}"#,
        );
        let items = vec![item(10, "q0", "a0"), item(20, "q1", "a1")];
        let topo = generate_topology(&items, None, &ask_config(&cli)).unwrap();
        assert_eq!("by topic", topo.principle);
        assert_eq!(vec![10, 20], topo.walk);
        assert_eq!(1, topo.edges.len());
        assert_eq!(10, topo.edges[0].from);
        assert_eq!(20, topo.edges[0].to);
        assert_eq!("leads to", topo.edges[0].label);
    }

    #[test]
    fn generate_topology_drops_out_of_range_indices() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // Index 5 doesn't exist (only 0 and 1), so it's dropped from the edge and
        // from the walk rather than failing the whole call.
        let cli = fake_reply(
            dir.path(),
            r#"{"principle":"p","edges":[{"from":0,"to":5,"label":"l"}],"walk":[0,5,1]}"#,
        );
        let items = vec![item(10, "q", "a"), item(20, "q", "a")];
        let topo = generate_topology(&items, None, &ask_config(&cli)).unwrap();
        assert_eq!(vec![10, 20], topo.walk);
        assert!(topo.edges.is_empty());
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
    fn generate_topology_names_auto_when_unguided() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"principle":"p","edges":[],"walk":[0]}"#);
        let unguided = generate_topology(&[item(10, "q", "a")], None, &ask_config(&cli)).unwrap();
        assert_eq!("auto", unguided.name);
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
    fn generate_topology_parses_regions_and_maps_card_indices() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(
            dir.path(),
            r#"{"principle":"p","edges":[],"walk":[0,1],"regions":[{"name":"Start","cards":[0]},{"name":"End","cards":[1]}]}"#,
        );
        let items = vec![item(10, "q0", "a0"), item(20, "q1", "a1")];
        let topo = generate_topology(&items, None, &ask_config(&cli)).unwrap();
        assert_eq!(2, topo.regions.len());
        assert_eq!("Start", topo.regions[0].name);
        assert_eq!(vec![10], topo.regions[0].cards);
        assert_eq!(vec![20], topo.regions[1].cards);
    }

    #[test]
    fn topology_order_from_walk_ranks_present_and_misses_absent() {
        let order = TopologyOrder::from_walk(&[10, 20, 30]);
        assert_eq!(Some(0), order.rank_of(10));
        assert_eq!(Some(2), order.rank_of(30));
        assert_eq!(None, order.rank_of(99));
    }

    #[test]
    fn run_config_clears_tools_and_applies_ai_overrides() {
        let ask = AskConfig {
            model: Some("sonnet".into()),
            allowed_tools: vec!["WebFetch".into()],
            ..AskConfig::default()
        };
        let ai = AiConfig {
            model: Some("haiku".into()),
            distractor_count: 3,
            variant_count: 4,
            timeout_secs: 42,
        };
        let cfg = run_config(&ai, &ask);
        assert!(cfg.allowed_tools.is_empty());
        assert_eq!(Some("haiku".to_string()), cfg.model);
        assert_eq!(42, cfg.timeout_secs);
    }

    #[test]
    fn run_config_falls_back_to_the_ask_model() {
        let ask = AskConfig {
            model: Some("sonnet".into()),
            ..AskConfig::default()
        };
        let cfg = run_config(&AiConfig::default(), &ask);
        assert_eq!(Some("sonnet".to_string()), cfg.model);
    }
}
