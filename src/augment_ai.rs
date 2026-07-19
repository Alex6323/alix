//! The AI generators for the augment cache: distractors, notes, key points,
//! variants, topology, format. Split out of `augment` so the cache/read side
//! compiles without the AI backend.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        mpsc::{Receiver, channel},
    },
};

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::Deserialize;

use crate::{
    answer::Mode,
    ask,
    augment::{Format, Topology, TopologyEdge, TopologyRegion, WarmItem},
    config::{AiConfig, AskConfig},
};

// ── Batch conversations ──────────────────────────────────────────────────────
//
// A batch of augmentations re-reads the same cards once per target when every
// call is a stateless one-shot. On a backend that keeps sessions (Claude), the
// batch instead primes ONE conversation with the card roster and runs each
// target as a `--resume` follow-up referencing cards by roster index, so the
// cards travel once.

/// One CLI conversation spanning a batch of augmentations: the session plus
/// the card roster its first call sends. Plain cloneable data: the batch owner
/// keeps it across polls, hands a snapshot into each spawned job, and flips
/// `session.started` (or [`reset`](Self::reset)s) when the job reports back.
#[derive(Clone)]
pub struct BatchConversation {
    /// The CLI session; `started` decides between priming and resuming.
    pub session: ask::CliSession,
    /// The batch's card roster, index-stable for the whole conversation.
    roster: Arc<Vec<WarmItem>>,
    /// Card id to roster index, for referencing a call's subset.
    index_of: Arc<HashMap<String, usize>>,
}

impl BatchConversation {
    /// A conversation over `roster`, or `None` when the configured backend
    /// keeps no sessions (only Claude does); callers then pass `None` through
    /// and every call stays a stateless one-shot, exactly as before.
    pub fn new(cfg: &AskConfig, roster: Vec<WarmItem>) -> Option<Self> {
        let keeps_session = crate::backend::backend_for(cfg)
            .map(|b| b.supports_session())
            .unwrap_or(false);
        if !keeps_session || roster.is_empty() {
            return None;
        }
        let index_of = roster
            .iter()
            .enumerate()
            .map(|(index, item)| (item.id.clone(), index))
            .collect();
        Some(Self {
            session: ask::CliSession::new(),
            roster: Arc::new(roster),
            index_of: Arc::new(index_of),
        })
    }

    /// Forgets the CLI session after a failed call (never `--resume` a session
    /// in an unknown state, mirroring the tutor's error arm). The roster stays,
    /// so the next call primes a fresh conversation with it.
    pub fn reset(&mut self) {
        self.session = ask::CliSession::new();
    }

    /// The roster indices of `items`, or `None` if any item is missing from
    /// the roster (that call then degrades to a stateless one-shot).
    fn indices_of(&self, items: &[WarmItem]) -> Option<Vec<usize>> {
        items
            .iter()
            .map(|item| self.index_of.get(&item.id).copied())
            .collect()
    }
}

/// The primer a conversation's first call sends: the whole roster, numbered,
/// in a shape every target's follow-up can reference.
fn roster_block(roster: &[WarmItem]) -> String {
    let mut s = String::from(
        "You will perform a series of augmentation tasks over one shared set \
         of flashcards in this conversation. Every task refers to these cards \
         by their index numbers.\n\
         Cards (index. FRONT / ANSWER / NOTE):\n",
    );
    s.push_str(&front_answer_note_lines(roster));
    s.push('\n');
    s
}

/// The card block for a call inside a conversation: reference the primed
/// roster by index instead of re-listing the cards.
fn reference_block(indices: &[usize], roster_len: usize) -> String {
    let mut s = String::from(
        "\nWork on the numbered flashcards already provided in this \
         conversation. ",
    );
    if indices.len() == roster_len {
        s.push_str("This task covers ALL the numbered cards.\n");
    } else {
        let list = indices
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        s.push_str(&format!(
            "This task covers the cards with these indices: {list}.\n"
        ));
    }
    s
}

/// Runs one generation call for `items` and returns the raw reply plus the
/// JSON key each item's answer arrives under. Stateless (no conversation, or
/// an item outside the roster): the prompt carries `listing` and keys are the
/// items' positions. In a conversation: the first turn is prefixed with the
/// roster primer, the card block references roster indices, and keys are those
/// indices. The tool allowlist is cleared either way (generation is a pure
/// text call, like exam remediation).
fn call_for_items(
    items: &[WarmItem],
    listing: &str,
    prompt_for: impl Fn(&str) -> String,
    conversation: Option<&BatchConversation>,
    ask_cfg: &AskConfig,
) -> Result<(String, Vec<String>)> {
    let cfg = tool_free(ask_cfg);
    if let Some(conversation) = conversation
        && let Some(indices) = conversation.indices_of(items)
    {
        let mut prompt = String::new();
        if !conversation.session.started {
            prompt.push_str(&roster_block(&conversation.roster));
        }
        prompt.push_str(&prompt_for(&reference_block(
            &indices,
            conversation.roster.len(),
        )));
        // `run_config` never sets a cwd, so every call in a batch shares this
        // process's directory and a plain `args()` resume finds the session.
        let raw = ask::run(&cfg, &prompt, &conversation.session.args())?;
        let keys = indices.iter().map(usize::to_string).collect();
        return Ok((raw, keys));
    }
    let raw = ask::run(&cfg, &prompt_for(listing), &[])?;
    Ok((raw, (0..items.len()).map(|i| i.to_string()).collect()))
}

// ── Generation ───────────────────────────────────────────────────────────────
//
// Distractors come from one batched, tool-free Claude call over the cards that
// still need them, mirroring the exam's generate/grade shape: a synchronous core
// ([`generate`]) the interactive frontends run on a thread via [`spawn`]. The
// call is pure text transformation — no web or file tools — so its allowlist is
// cleared like exam remediation.

/// Generates up to `count` distractors per card in `items` with one batched,
/// tool-free call, optionally steered by `guidance` (the `--with` text). Returns
/// a map from card id to its validated distractors; cards the model produced
/// nothing usable for are omitted, so review falls back to offline sampling.
pub fn generate(
    items: &[WarmItem],
    count: usize,
    guidance: Option<&str>,
    ask_cfg: &AskConfig,
    conversation: Option<&BatchConversation>,
) -> Result<HashMap<String, Vec<String>>> {
    if items.is_empty() {
        return Ok(HashMap::new());
    }
    let listing = question_answer_listing(items, "index. question — correct answer");
    let (raw, keys) = call_for_items(
        items,
        &listing,
        |cards| distractors_prompt(cards, count, guidance),
        conversation,
        ask_cfg,
    )?;
    let parsed: HashMap<String, Vec<String>> =
        parse_json(&raw).context("parsing the generated distractors")?;

    let mut out = HashMap::new();
    for (key, item) in keys.iter().zip(items) {
        let Some(raw_options) = parsed.get(key) else {
            continue;
        };
        let cleaned = clean_distractors(raw_options, &item.answer, count);
        if !cleaned.is_empty() {
            out.insert(item.id.clone(), cleaned);
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
    conversation: Option<&BatchConversation>,
) -> Result<HashMap<String, String>> {
    if items.is_empty() {
        return Ok(HashMap::new());
    }
    let listing = question_answer_listing(items, "index. question — answer");
    let (raw, keys) = call_for_items(
        items,
        &listing,
        |cards| notes_prompt(cards, guidance),
        conversation,
        ask_cfg,
    )?;
    let parsed: HashMap<String, String> =
        parse_json(&raw).context("parsing the generated notes")?;

    let mut out = HashMap::new();
    for (key, item) in keys.iter().zip(items) {
        if let Some(note) = parsed.get(key) {
            let note = note.trim();
            if !note.is_empty() {
                out.insert(item.id.clone(), note.to_string());
            }
        }
    }
    Ok(out)
}

/// Decomposes each card's answer into the few load-bearing, independently
/// checkable claims (the Explain-mode checklist rubric), up to `count`, optionally
/// steered by `guidance`. Returns card id → its key points, **omitting a card
/// whose answer is atomic** — fewer than two claims means there's nothing to check
/// off one by one, so the card keeps its plain self-graded reveal (just as choice
/// mode omits cards with no usable distractor).
pub fn generate_keypoints(
    items: &[WarmItem],
    count: usize,
    guidance: Option<&str>,
    ask_cfg: &AskConfig,
    conversation: Option<&BatchConversation>,
) -> Result<HashMap<String, Vec<String>>> {
    if items.is_empty() {
        return Ok(HashMap::new());
    }
    let listing = question_answer_listing(items, "index. question — answer");
    let (raw, keys) = call_for_items(
        items,
        &listing,
        |cards| keypoints_prompt(cards, count, guidance),
        conversation,
        ask_cfg,
    )?;
    let parsed: HashMap<String, Vec<String>> =
        parse_json(&raw).context("parsing the generated key points")?;

    let mut out = HashMap::new();
    for (key, item) in keys.iter().zip(items) {
        let Some(raw_points) = parsed.get(key) else {
            continue;
        };
        let cleaned = clean_keypoints(raw_points, count);
        // An atomic answer yields fewer than two checkable claims — nothing to
        // tick off — so omit it; the card keeps its plain self-graded reveal.
        if cleaned.len() >= 2 {
            out.insert(item.id.clone(), cleaned);
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
    conversation: Option<&BatchConversation>,
) -> Result<HashMap<String, Vec<String>>> {
    if items.is_empty() {
        return Ok(HashMap::new());
    }
    let listing = question_answer_listing(items, "index. question — the answer it must still have");
    let (raw, keys) = call_for_items(
        items,
        &listing,
        |cards| variants_prompt(cards, count, guidance),
        conversation,
        ask_cfg,
    )?;
    let parsed: HashMap<String, Vec<String>> =
        parse_json(&raw).context("parsing the generated question variants")?;

    let mut out = HashMap::new();
    for (key, item) in keys.iter().zip(items) {
        let Some(raw_variants) = parsed.get(key) else {
            continue;
        };
        let cleaned = clean_variants(raw_variants, &item.question, count);
        if !cleaned.is_empty() {
            out.insert(item.id.clone(), cleaned);
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
    deck_token: &str,
    ask_cfg: &AskConfig,
    conversation: Option<&BatchConversation>,
) -> Result<Topology> {
    if items.is_empty() {
        return Ok(Topology::default());
    }
    let listing = question_answer_listing(items, "index. question — answer");
    let (raw, keys) = call_for_items(
        items,
        &listing,
        |cards| topology_prompt(cards, guidance),
        conversation,
        ask_cfg,
    )?;
    let parsed: RawTopology = parse_json(&raw).context("parsing the generated topology")?;
    // The indices embedded in the reply (edges, walk, regions) are the same
    // ones the cards were listed under: positions stateless, roster indices in
    // a conversation. The keys give exactly that mapping per item.
    let key_ids: HashMap<usize, String> = keys
        .iter()
        .zip(items)
        .filter_map(|(key, item)| {
            key.parse::<usize>()
                .ok()
                .map(|index| (index, item.id.clone()))
        })
        .collect();
    let mut topology = to_topology(parsed, &key_ids);
    topology.name = guidance
        .map(|g| g.trim())
        .filter(|g| !g.is_empty())
        .unwrap_or("pedagogical order")
        .to_string();
    // Bind the topology to the deck it was built over, so a shared cache never
    // leaks it onto another deck (and a moved card never drags it along).
    topology.deck_token = deck_token.to_string();
    Ok(topology)
}

/// Maps a [`RawTopology`]'s card indices back to identity hashes via `ids`
/// (index under which a card was listed, to its id), dropping any unknown
/// index and any card repeated in the walk.
fn to_topology(raw: RawTopology, ids: &HashMap<usize, String>) -> Topology {
    let id_of = |idx: usize| ids.get(&idx).cloned();
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
        .filter(|id| seen.insert(id.clone()))
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
        // The owner deck token is stamped by the caller (`generate_topology`).
        deck_token: String::new(),
    }
}

/// Builds the topology prompt: the instructions, then `cards_block` (a listing
/// or a conversation reference), asking for an organizing principle, a labeled
/// edge set, and a walk that visits every card so consecutive ones relate.
fn topology_prompt(cards_block: &str, guidance: Option<&str>) -> String {
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
         Use the cards' index numbers. Relate cards by their meaning, not their \
         wording.\n",
    );
    match guidance.map(str::trim).filter(|g| !g.is_empty()) {
        Some(g) => s.push_str(&format!("\nFavored organizing principle: {g}\n")),
        None => s.push_str(
            "\nFavored organizing principle: a pedagogical order that puts \
             foundational cards first, then the cards that build on them, so \
             prerequisites come before what depends on them.\n",
        ),
    }
    s.push_str(cards_block);
    s.push_str(
        "\nOutput ONLY JSON in exactly this shape, no prose, no code fences:\n\
         {\"principle\": \"...\", \
         \"edges\": [{\"from\": 0, \"to\": 1, \"label\": \"...\"}], \
         \"walk\": [0, 1, ...], \
         \"regions\": [{\"name\": \"...\", \"cards\": [0, 1]}]}\n",
    );
    s
}

/// The model's raw reshape for one card (mode is a string here; validated into a
/// `Mode` by `clean_format`). All fields optional — the model omits a field it
/// leaves unchanged, and omits the whole entry for an already-clean card.
#[derive(Deserialize)]
struct RawFormat {
    #[serde(default)]
    front: Option<String>,
    #[serde(default)]
    back: Vec<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    mode: Option<String>,
}

/// Validates one raw reshape against the card it came from, returning a `Format`
/// only if it is a real, usable improvement. Trims fields and drops empty ones;
/// accepts a suggested mode only if it parses and is a self-graded/reveal mode
/// (`flip`/`line`) — never an exact-match mode that the reshaped lines
/// would break; requires the reshape to actually differ from the original.
fn clean_format(item: &WarmItem, raw: &RawFormat) -> Option<Format> {
    let front = raw
        .front
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && *s != item.question.trim());

    let back: Vec<String> = raw
        .back
        .iter()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    // Keep the reshaped answer only if it differs from the original lines.
    let original_lines: Vec<&str> = item.answer.lines().map(str::trim).collect();
    let back = if !back.is_empty() && back.iter().map(String::as_str).ne(original_lines) {
        back
    } else {
        Vec::new()
    };

    let note = raw
        .note
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && item.note.as_deref().map(str::trim) != Some(s.as_str()));

    let mode = raw
        .mode
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| Mode::from_str(s, true).ok())
        .filter(|m| matches!(m, Mode::Flip | Mode::LineByLine));

    if front.is_none() && back.is_empty() && note.is_none() && mode.is_none() {
        return None;
    }
    Some(Format {
        front,
        back,
        note,
        mode,
    })
}

/// Reshapes badly-shaped cards with one batched, tool-free call: returns a map
/// from card id to its validated `Format`. Cards the model judged already clean
/// (or returned nothing usable for) are omitted.
pub fn generate_format(
    items: &[WarmItem],
    guidance: Option<&str>,
    ask_cfg: &AskConfig,
    conversation: Option<&BatchConversation>,
) -> Result<HashMap<String, Format>> {
    if items.is_empty() {
        return Ok(HashMap::new());
    }
    let listing = format!(
        "\nCards (index. FRONT / ANSWER / NOTE):\n{}",
        front_answer_note_lines(items)
    );
    let (raw, keys) = call_for_items(
        items,
        &listing,
        |cards| format_prompt(cards, guidance),
        conversation,
        ask_cfg,
    )?;
    let parsed: HashMap<String, RawFormat> =
        parse_json(&raw).context("parsing the generated card formats")?;

    let mut out = HashMap::new();
    for (key, item) in keys.iter().zip(items) {
        // A card the model reshapes gets its tidied Format; one it declines
        // (already clean, so omitted from the reply) gets an all-empty no-op
        // Format. The no-op still counts as covered, so a well-shaped card is
        // marked done instead of lingering as a gap that re-runs to no effect.
        let fmt = parsed
            .get(key)
            .and_then(|raw_fmt| clean_format(item, raw_fmt))
            .unwrap_or_default();
        out.insert(item.id.clone(), fmt);
    }
    Ok(out)
}

fn format_prompt(cards_block: &str, guidance: Option<&str>) -> String {
    let mut s = String::from(
        "You improve the PRESENTATION of flashcards. For each card decide whether \
         it is badly shaped — most often a list of several items crammed into one \
         prose answer, or a dense unreadable answer/question. If it is, return a \
         tidied version; if it is already clean or atomic, OMIT it entirely.\n\n\
         Rules:\n\
         - Only surface structure that is already there (an enumeration, groups, \
         ordered steps, embedded code). NEVER invent structure or pad an atomic \
         answer into a list.\n\
         - `back` is the answer as display lines: one item or group per line; \
         keep the same facts and words, only regroup/relabel for clarity.\n\
         - In the OUTPUT, do not wrap terms in inline backticks — write a name like \
         Foo::bar plainly. Single-backtick code spans read as visual noise on a card.\n\
         - A real code snippet (more than a short token) goes in a fenced block \
         inside `back`: a line ```lang, then the code lines indented the way the \
         language wants, then a closing ``` line — best-effort on the language tag.\n\
         - `front`/`note`: reshape only for readability. The question's layout must \
         NOT leak the answer (never hint how many items it has).\n\
         - `mode`: suggest either `flip` or `line` ONLY when it fits the reshaped \
         answer (use `line` for an ordered/grouped list revealed one line at a \
         time). Never suggest explain/typing/typeline/choice. Omit `mode` if unsure.\n\
         - Omit any field you leave unchanged; omit the whole card if it is fine.\n",
    );
    if let Some(g) = guidance {
        s.push_str(&format!("\nExtra guidance: {}\n", g.trim()));
    }
    s.push_str(cards_block);
    s.push_str(
        "\nOutput ONLY JSON, no prose, and no markdown fence around the JSON itself \
         (a code snippet inside a `back` string may still be fenced). The key is the card index; \
         the value is an object with any of \"front\" (string), \"back\" (array of \
         strings), \"note\" (string), \"mode\" (string). Include only cards that \
         need reshaping:\n\
         {\"0\": {\"back\": [\"...\", \"...\"], \"mode\": \"line\"}, ...}\n",
    );
    s
}

// ── Background generation ──────────────────────────────────────────────────────
//
// The web server can't block its request loop on a costed Claude call, so it
// runs generation on a thread and polls the returned channel — the same shape
// as `ask::spawn` and `trace_ai::spawn_grade`. The worker only *generates*; the
// caller applies the [`Outcome`] to the cache and saves, keeping cache writes
// single-threaded.

/// A generation request for one target. Per-card targets carry the gap items the
/// caller computed (e.g. via [`AugmentCache::missing_choices`]); topology is
/// whole-deck.
pub enum Job {
    Choices {
        items: Vec<WarmItem>,
        count: usize,
    },
    Notes {
        items: Vec<WarmItem>,
    },
    Questions {
        items: Vec<WarmItem>,
        count: usize,
    },
    Keypoints {
        items: Vec<WarmItem>,
        count: usize,
    },
    Topology {
        items: Vec<WarmItem>,
        /// The owner deck's identity token, stamped onto the generated topology
        /// so a shared cache keeps each deck's topologies apart.
        deck_token: String,
    },
    Format {
        items: Vec<WarmItem>,
    },
    /// Draw the workspace emblem at `dir` (the workspace augment screen's
    /// icon target); `guidance` steers the style.
    Icon {
        dir: std::path::PathBuf,
    },
}

/// The result of a [`Job`], shaped per target so the caller can apply it to the
/// cache (`set_distractors` / `set_note` / … or `add_topology`).
pub enum Outcome {
    Choices(HashMap<String, Vec<String>>),
    Notes(HashMap<String, String>),
    Questions(HashMap<String, Vec<String>>),
    Keypoints(HashMap<String, Vec<String>>),
    Topology(Topology),
    Format(HashMap<String, Format>),
    /// The freshly written workspace icon. Nothing to cache — the file on
    /// disk is the result.
    Icon(std::path::PathBuf),
}

/// Runs a generation [`Job`] on a background thread; the [`Outcome`] (or an error
/// message) arrives on the returned channel, which the caller polls with
/// `try_recv`. `guidance` is the `--with` steer; `conversation` is the batch's
/// shared conversation snapshot (None: a stateless one-shot).
pub fn spawn(
    job: Job,
    guidance: Option<String>,
    ask_cfg: AskConfig,
    conversation: Option<BatchConversation>,
) -> Receiver<Result<Outcome, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let reply = run_job(job, guidance.as_deref(), &ask_cfg, conversation.as_ref())
            .map_err(|e| format!("{e:#}"));
        // The receiver may be gone if the user left the Augment screen.
        let _ = tx.send(reply);
    });
    rx
}

/// The synchronous core of [`spawn`]: dispatches to the matching `generate_*`.
fn run_job(
    job: Job,
    guidance: Option<&str>,
    ask_cfg: &AskConfig,
    conversation: Option<&BatchConversation>,
) -> Result<Outcome> {
    Ok(match job {
        Job::Choices { items, count } => {
            Outcome::Choices(generate(&items, count, guidance, ask_cfg, conversation)?)
        }
        Job::Notes { items } => {
            Outcome::Notes(generate_notes(&items, guidance, ask_cfg, conversation)?)
        }
        Job::Questions { items, count } => Outcome::Questions(generate_variants(
            &items,
            count,
            guidance,
            ask_cfg,
            conversation,
        )?),
        Job::Keypoints { items, count } => Outcome::Keypoints(generate_keypoints(
            &items,
            count,
            guidance,
            ask_cfg,
            conversation,
        )?),
        Job::Topology { items, deck_token } => Outcome::Topology(generate_topology(
            &items,
            guidance,
            &deck_token,
            ask_cfg,
            conversation,
        )?),
        Job::Format { items } => {
            Outcome::Format(generate_format(&items, guidance, ask_cfg, conversation)?)
        }
        // Card-free and prompt-owning: an icon draw never rides (or disturbs)
        // the batch conversation.
        Job::Icon { dir } => Outcome::Icon(crate::icon::generate(&dir, guidance, ask_cfg)?),
    })
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

/// Builds the batched distractor prompt: the instructions, then `cards_block`
/// (a listing or a conversation reference), then a strict JSON output shape
/// keyed by card index.
fn distractors_prompt(cards_block: &str, count: usize, guidance: Option<&str>) -> String {
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
    s.push_str(cards_block);
    let slots = vec!["\"...\""; count].join(", ");
    s.push_str(&format!(
        "\nOutput ONLY JSON in exactly this shape, no prose, no code fences — \
         the key is the card index, the value its {count} distractors:\n\
         {{\"0\": [{slots}], ...}}\n"
    ));
    s
}

/// Builds the batched notes prompt: one short note per card, keyed by index.
fn notes_prompt(cards_block: &str, guidance: Option<&str>) -> String {
    let mut s = String::from(
        "You are adding a short note to each flashcard — one or two sentences of \
         memorable trivia, context, or a mnemonic that makes the answer easier to \
         recall. Keep each note tight and factual, and do not simply restate the \
         answer.\n",
    );
    if let Some(g) = guidance {
        s.push_str(&format!("\nExtra guidance: {}\n", g.trim()));
    }
    s.push_str(cards_block);
    s.push_str(
        "\nOutput ONLY JSON, no prose, no code fences — the key is the card index, \
         the value its note as a single string:\n{\"0\": \"...\", ...}\n",
    );
    s
}

/// Builds the batched key-points prompt: decompose each answer into its
/// load-bearing claims, keyed by index. Atomic answers return an empty list so
/// they aren't forced into a meaningless single "point".
fn keypoints_prompt(cards_block: &str, count: usize, guidance: Option<&str>) -> String {
    let mut s = format!(
        "Break each flashcard's ANSWER into its load-bearing claims — a checklist \
         a learner ticks off after recalling the card. Give at most {count} per card.\n\
         Condense each point to the BARE MINIMUM: a 2–5 word telegraphic tag, not a \
         sentence and not a rephrasing of the answer. Drop articles, verbs of being, \
         and any word the point survives without — but ALWAYS keep the relation that \
         carries the claim (a comparison like \"more than\", \">\", or \"beats\"; a \
         cause; an order; a negation): dropping it loses or inverts the meaning. \
         \"retrieval > re-study\" or \"retrieval beats re-study\" is right; \
         \"retrieval, re-study\" is wrong.\n\
         - One distinct idea per point; points must be independent (none restates \
         another).\n\
         - Use ONLY what the answer states; invent nothing.\n\
         - If the answer is atomic — a single fact, term, name, number, or date with \
         nothing to decompose — return an EMPTY list. Never pad it into one point.\n\
         Example — answer: \"Reviewing a card just before you would forget it forces \
         effortful retrieval, which strengthens memory far more than re-reading; \
         spacing keeps reviews near that forgetting edge, while cramming only loads \
         short-term memory that soon fades.\"\n\
         GOOD (condensed): [\"retrieval > re-study\", \"effortful recall\", \"timed \
         near forgetting\", \"cramming = short-term only\"]\n\
         BAD (too wordy): [\"Reviewing just before you forget forces effortful \
         retrieval that strengthens memory more than re-reading\"]\n"
    );
    if let Some(g) = guidance {
        s.push_str(&format!("\nExtra guidance: {}\n", g.trim()));
    }
    s.push_str(cards_block);
    s.push_str(
        "\nOutput ONLY JSON, no prose, no code fences — the key is the card index, \
         the value its list of key points (an empty list for an atomic answer):\n\
         {\"0\": [\"...\", \"...\"], \"1\": [], ...}\n",
    );
    s
}

/// Builds the batched variants prompt: rephrase each question, keep the answer.
fn variants_prompt(cards_block: &str, count: usize, guidance: Option<&str>) -> String {
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
    s.push_str(cards_block);
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

/// Trims, drops empties, dedups (case-insensitively), and caps at `count`.
fn clean_keypoints(raw: &[String], count: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for point in raw {
        let trimmed = point.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_lowercase()) {
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

/// The stateless question/answer card listing under `heading` (each target
/// keeps its own heading wording).
fn question_answer_listing(items: &[WarmItem], heading: &str) -> String {
    let mut s = format!("\nCards ({heading}):\n");
    for (i, item) in items.iter().enumerate() {
        s.push_str(&format!(
            "{i}. {} — {}\n",
            one_line(&item.question),
            one_line(&item.answer)
        ));
    }
    s
}

/// One `index. FRONT / ANSWER / NOTE` line per card, shared by the format
/// target's listing and the conversation primer.
fn front_answer_note_lines(items: &[WarmItem]) -> String {
    let mut s = String::new();
    for (i, item) in items.iter().enumerate() {
        s.push_str(&format!(
            "{i}. {} / {} / {}\n",
            one_line(&item.question),
            one_line(&item.answer),
            item.note.as_deref().map(one_line).unwrap_or_default()
        ));
    }
    s
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
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::{
        config::BackendKind,
        testutil::{ask_config, exec_lock, fake_cli, fake_reply},
    };

    // ── generation ──

    fn item(id: u64, question: &str, answer: &str) -> WarmItem {
        WarmItem {
            id: id.to_string(),
            question: question.into(),
            answer: answer.into(),
            note: None,
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
        let out = generate(&items, 3, None, &ask_config(&cli), None).unwrap();
        assert_eq!(vec!["w1", "w2", "w3"], out["10"]);
        assert_eq!(vec!["x1", "x2", "x3"], out["20"]);
    }

    #[test]
    fn augment_never_touches_the_deck_file() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("d.md");
        let deck_src = "---\nid: \"9w2c7x4k1m8q3z5t0v6b2n4d8f\"\n---\n## Capital of France? <!-- id: 4jkya9q3m8z0tw5v9y2b4n6d8f -->\nParis\n";
        std::fs::write(&deck_path, deck_src).unwrap();
        let before = std::fs::read(&deck_path).unwrap();

        let deck = crate::l1::parse_l1("d.md", deck_src).unwrap();
        let items: Vec<WarmItem> = deck.cards.iter().map(WarmItem::from_card).collect();
        let cli = fake_reply(dir.path(), r#"{"0": ["w1","w2","w3"]}"#);
        let map = generate(&items, 3, None, &ask_config(&cli), None).unwrap();

        let store_path = dir.path().join("progress.json");
        let cache_path = crate::augment::augment_path_for(&store_path);
        let mut cache = crate::augment::AugmentCache::open(&cache_path);
        for (id, distractors) in &map {
            cache.set_distractors(id, distractors.clone());
        }
        cache.save().unwrap();

        assert_eq!(before, std::fs::read(&deck_path).unwrap());
        assert!(cache_path.exists());
    }

    #[test]
    fn generate_keypoints_parses_and_maps_each_card() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"0": ["it moves a", "use_it owns a"]}"#);
        let items = vec![item(10, "What happens to a?", "a is moved into use_it")];
        let out = generate_keypoints(&items, 5, None, &ask_config(&cli), None).unwrap();
        assert_eq!(vec!["it moves a", "use_it owns a"], out["10"]);
    }

    #[test]
    fn generate_keypoints_omits_an_atomic_card() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // The model returns an empty list for the atomic card and a real
        // decomposition for the conceptual one — only the latter is kept.
        let cli = fake_reply(dir.path(), r#"{"0": [], "1": ["claim a", "claim b"]}"#);
        let items = vec![
            item(10, "Capital of France?", "Paris"),
            item(20, "How does X work?", "first A, then B"),
        ];
        let out = generate_keypoints(&items, 5, None, &ask_config(&cli), None).unwrap();
        assert!(!out.contains_key("10")); // atomic → omitted
        assert_eq!(vec!["claim a", "claim b"], out["20"]);
    }

    #[test]
    fn generate_keypoints_omits_a_single_point_as_atomic() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // One lone point isn't a checklist — treated as atomic and dropped.
        let cli = fake_reply(dir.path(), r#"{"0": ["the only claim"]}"#);
        let items = vec![item(10, "Q?", "a single fact")];
        let out = generate_keypoints(&items, 5, None, &ask_config(&cli), None).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn generate_keypoints_malformed_json_is_an_error() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), "not json at all");
        let items = vec![item(10, "Q?", "A")];
        assert!(generate_keypoints(&items, 5, None, &ask_config(&cli), None).is_err());
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
            None,
        )
        .unwrap();
        assert_eq!(vec!["Lyon", "Nice"], out["1"]);
    }

    #[test]
    fn generate_caps_at_count() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"0": ["a","b","c","d","e"]}"#);
        let out = generate(&[item(1, "q", "z")], 3, None, &ask_config(&cli), None).unwrap();
        assert_eq!(3, out["1"].len());
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
            None,
        )
        .unwrap();
        assert!(!out.contains_key("1"));
        assert_eq!(vec!["x1"], out["2"]);
    }

    #[test]
    fn generate_malformed_json_is_an_error() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), "sorry, I can't do that");
        let err = generate(&[item(1, "q", "a")], 3, None, &ask_config(&cli), None).unwrap_err();
        assert!(format!("{err:#}").contains("valid JSON"));
    }

    #[test]
    fn generate_with_no_items_makes_no_call() {
        // No real CLI: empty input must short-circuit to an empty map.
        let cfg = ask_config(Path::new("/nonexistent/claude"));
        assert!(generate(&[], 3, None, &cfg, None).unwrap().is_empty());
    }

    #[test]
    fn generate_notes_parses_each_card() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"0": "note a", "1": "note b"}"#);
        let items = vec![item(10, "q1", "a1"), item(20, "q2", "a2")];
        let out = generate_notes(&items, None, &ask_config(&cli), None).unwrap();
        assert_eq!("note a", out["10"]);
        assert_eq!("note b", out["20"]);
    }

    #[test]
    fn generate_notes_omits_blank_notes() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"0": "   ", "1": "real note"}"#);
        let items = vec![item(1, "q", "a"), item(2, "q", "a")];
        let out = generate_notes(&items, None, &ask_config(&cli), None).unwrap();
        assert!(!out.contains_key("1"));
        assert_eq!("real note", out["2"]);
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
        let out = generate_variants(
            &[item(1, "What year?", "1589")],
            3,
            None,
            &ask_config(&cli),
            None,
        )
        .unwrap();
        assert_eq!(vec!["In which year?", "Which year was it?"], out["1"]);
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
        let topo = generate_topology(&items, None, "dtok", &ask_config(&cli), None).unwrap();
        assert_eq!("by topic", topo.principle);
        assert_eq!(topo.walk, ["10", "20"]);
        assert_eq!(1, topo.edges.len());
        assert_eq!(topo.edges[0].from, "10");
        assert_eq!(topo.edges[0].to, "20");
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
        let topo = generate_topology(&items, None, "dtok", &ask_config(&cli), None).unwrap();
        assert_eq!(topo.walk, ["10", "20"]);
        assert!(topo.edges.is_empty());
    }

    #[test]
    fn generate_topology_names_it_pedagogical_order_when_unguided() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"principle":"p","edges":[],"walk":[0]}"#);
        let unguided =
            generate_topology(&[item(10, "q", "a")], None, "dtok", &ask_config(&cli), None)
                .unwrap();
        assert_eq!("pedagogical order", unguided.name);
    }

    #[test]
    fn topology_prompt_defaults_to_a_pedagogical_order_and_guidance_overrides_it() {
        let items = [item(1, "q", "a")];
        let listing = question_answer_listing(&items, "index. question — answer");
        let unguided = topology_prompt(&listing, None);
        assert!(unguided.contains("pedagogical order"), "{unguided}");
        assert!(unguided.contains("foundational cards first"), "{unguided}");
        let guided = topology_prompt(&listing, Some("by continent"));
        assert!(
            guided.contains("Favored organizing principle: by continent"),
            "{guided}"
        );
        assert!(!guided.contains("foundational cards first"), "{guided}");
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
        let topo = generate_topology(&items, None, "dtok", &ask_config(&cli), None).unwrap();
        assert_eq!(2, topo.regions.len());
        assert_eq!("Start", topo.regions[0].name);
        assert_eq!(topo.regions[0].cards, ["10"]);
        assert_eq!(topo.regions[1].cards, ["20"]);
    }

    #[test]
    fn topology_in_a_conversation_remaps_embedded_indices_through_the_roster() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // The model's reply addresses cards by ROSTER index (what the primer
        // showed), not by their position in `items` below — the roster is
        // deliberately ordered differently from `items` so a positional
        // (rather than roster-lookup) remapping would silently misattribute.
        let cli = fake_reply(
            dir.path(),
            r#"{"principle":"p","edges":[{"from":0,"to":1,"label":"l"}],"walk":[0,1,2]}"#,
        );
        let cfg = ask_config(&cli);
        let roster = vec![
            item(30, "q30", "a30"),
            item(10, "q10", "a10"),
            item(20, "q20", "a20"),
        ];
        let conversation = BatchConversation::new(&cfg, roster).expect("claude keeps sessions");
        let items = vec![
            item(10, "q10", "a10"),
            item(20, "q20", "a20"),
            item(30, "q30", "a30"),
        ];
        let topo = generate_topology(&items, None, "dtok", &cfg, Some(&conversation)).unwrap();
        // Roster indices 0,1,2 are cards 30,10,20 — not the input order 10,20,30.
        assert_eq!(topo.walk, ["30", "10", "20"]);
        assert_eq!(topo.edges[0].from, "30");
        assert_eq!(topo.edges[0].to, "10");
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
            keypoint_count: 5,
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

    #[test]
    fn spawn_delivers_an_outcome_on_the_channel() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"0": ["w1","w2","w3"]}"#);
        let job = Job::Choices {
            items: vec![item(10, "Capital of France?", "Paris")],
            count: 3,
        };
        let rx = spawn(job, None, ask_config(&cli), None);
        match rx.recv().unwrap() {
            Ok(Outcome::Choices(map)) => assert_eq!(vec!["w1", "w2", "w3"], map["10"]),
            Ok(_) => panic!("expected a Choices outcome"),
            Err(e) => panic!("generation failed: {e}"),
        }
    }

    // ── batch conversations ──

    /// A fake CLI for multi-call conversations: appends each call's argv and
    /// prompt to numbered logs, then replies with the matching canned reply.
    fn fake_conversation(dir: &Path, replies: &[&str]) -> PathBuf {
        for (i, reply) in replies.iter().enumerate() {
            std::fs::write(dir.join(format!("reply-{i}")), reply).unwrap();
        }
        let d = dir.display();
        fake_cli(
            dir,
            &format!(
                "N=$(cat {d}/n 2>/dev/null || echo 0)\n\
                 echo \"$@\" >> {d}/args.log\n\
                 cat >> {d}/prompt-$N.log\n\
                 echo $((N+1)) > {d}/n\n\
                 cat {d}/reply-$N"
            ),
        )
    }

    #[test]
    fn a_conversation_primes_once_and_follows_up_by_index() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_conversation(
            dir.path(),
            &[r#"{"0": ["w1","w2","w3"]}"#, r#"{"1": "a note"}"#],
        );
        let cfg = ask_config(&cli);
        let roster = vec![
            item(10, "Capital of France?", "Paris"),
            item(20, "2+2?", "4"),
        ];
        let mut conversation =
            BatchConversation::new(&cfg, roster.clone()).expect("claude keeps sessions");

        // First target: choices for card 0 only (a subset) primes the roster.
        let out = generate(&roster[..1], 3, None, &cfg, Some(&conversation)).unwrap();
        assert_eq!(vec!["w1", "w2", "w3"], out["10"]);
        // what the batch owner does after a successful call
        conversation.session.started = true;

        // Second target: notes for card 1; the reply is keyed by ROSTER index.
        let out = generate_notes(&roster[1..], None, &cfg, Some(&conversation)).unwrap();
        assert_eq!("a note", out["20"]);

        let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
        let calls: Vec<&str> = args.lines().collect();
        assert_eq!(2, calls.len(), "{args}");
        assert!(calls[0].contains("--session-id"), "{args}");
        assert!(calls[1].contains("--resume"), "{args}");
        let id = calls[0]
            .split_whitespace()
            .skip_while(|w| *w != "--session-id")
            .nth(1)
            .expect("an id follows --session-id");
        assert!(calls[1].contains(id), "one conversation: {args}");

        let primer = std::fs::read_to_string(dir.path().join("prompt-0.log")).unwrap();
        let follow_up = std::fs::read_to_string(dir.path().join("prompt-1.log")).unwrap();
        assert!(
            primer.contains("Paris") && primer.contains("2+2?"),
            "the primer lists the whole roster: {primer}"
        );
        assert!(follow_up.contains("indices: 1"), "{follow_up}");
        assert!(
            !follow_up.contains("2+2?"),
            "a follow-up must not re-list cards: {follow_up}"
        );
    }

    #[test]
    fn conversation_replies_are_keyed_by_roster_index() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"0": ["x1"], "2": ["z1"]}"#);
        let cfg = ask_config(&cli);
        let roster = vec![
            item(10, "q0", "a0"),
            item(20, "q1", "a1"),
            item(30, "q2", "a2"),
        ];
        let conversation = BatchConversation::new(&cfg, roster).unwrap();
        // The call covers roster cards 0 and 2; keys "0"/"2" must land on
        // those cards, not on positions 0/1 of the subset.
        let items = vec![item(10, "q0", "a0"), item(30, "q2", "a2")];
        let out = generate(&items, 3, None, &cfg, Some(&conversation)).unwrap();
        assert_eq!(vec!["x1"], out["10"]);
        assert_eq!(vec!["z1"], out["30"]);
        assert!(!out.contains_key("20"));
    }

    #[test]
    fn a_sessionless_backend_gets_no_conversation() {
        let cfg = AskConfig {
            backend: BackendKind::Gemini,
            ..AskConfig::default()
        };
        assert!(BatchConversation::new(&cfg, vec![item(1, "q", "a")]).is_none());
    }

    #[test]
    fn an_item_outside_the_roster_degrades_to_a_stateless_call() {
        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_conversation(dir.path(), &[r#"{"0": ["x1"]}"#]);
        let cfg = ask_config(&cli);
        let conversation = BatchConversation::new(&cfg, vec![item(10, "q0", "a0")]).unwrap();
        // Card 99 is not in the roster: the call must re-list it and carry no
        // session flags (and position keys apply again).
        let items = vec![item(99, "Rogue?", "yes")];
        let out = generate(&items, 3, None, &cfg, Some(&conversation)).unwrap();
        assert_eq!(vec!["x1"], out["99"]);
        let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
        assert!(
            !args.contains("--session-id") && !args.contains("--resume"),
            "{args}"
        );
        let prompt = std::fs::read_to_string(dir.path().join("prompt-0.log")).unwrap();
        assert!(prompt.contains("Rogue?"), "{prompt}");
    }

    // ── format generation ──

    #[test]
    fn clean_format_keeps_a_real_reshape_and_validates_mode() {
        let item = WarmItem {
            id: "1".to_string(),
            question: "List the parts".to_string(),
            answer: "A, B, C".to_string(),
            note: None,
        };
        let raw = RawFormat {
            front: None,
            back: vec!["A".to_string(), "B".to_string(), "C".to_string()],
            note: None,
            mode: Some("line".to_string()),
        };
        let fmt = clean_format(&item, &raw).expect("a reshape");
        assert_eq!(fmt.back, ["A", "B", "C"]);
        assert_eq!(fmt.mode, Some(Mode::LineByLine));
    }

    #[test]
    fn clean_format_drops_noop_and_bad_mode() {
        let item = WarmItem {
            id: "1".to_string(),
            question: "Q".to_string(),
            answer: "A, B, C".to_string(),
            note: None,
        };
        // Same lines as the original answer, and an exact-match mode -> nothing usable.
        let raw = RawFormat {
            front: None,
            back: vec!["A, B, C".to_string()],
            note: None,
            mode: Some("typing".to_string()),
        };
        assert!(clean_format(&item, &raw).is_none());
    }

    #[test]
    fn generate_format_errors_on_non_json_reply() {
        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), "sorry, I can't do that");
        let items = vec![WarmItem {
            id: "1".to_string(),
            question: "List the parts".to_string(),
            answer: "A, B, C".to_string(),
            note: None,
        }];
        let err = generate_format(&items, None, &ask_config(&cli), None).unwrap_err();
        assert!(format!("{err:#}").contains("did not return valid JSON"));
    }

    #[test]
    fn generate_format_marks_a_declined_card_with_a_noop_so_it_stays_covered() {
        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // The model reshapes card 0 and omits card 1 as already clean.
        let cli = fake_reply(dir.path(), r#"{"0": {"back": ["A", "B"]}}"#);
        let items = vec![
            WarmItem {
                id: "1".to_string(),
                question: "List".into(),
                answer: "A, B".into(),
                note: None,
            },
            WarmItem {
                id: "2".to_string(),
                question: "Atomic".into(),
                answer: "one thing".into(),
                note: None,
            },
        ];
        let map = generate_format(&items, None, &ask_config(&cli), None).unwrap();
        assert_eq!(map["1"].back, ["A", "B"]);
        // The declined card gets an all-empty no-op so it counts as covered
        // instead of lingering as an eternal gap the user re-runs for nothing.
        assert!(map.contains_key("2"), "declined card missing");
        assert_eq!(map["2"], Format::default());
    }

    #[test]
    fn generate_format_parses_a_reshape() {
        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), r#"{"0": {"back": ["A", "B"], "mode": "line"}}"#);
        let items = vec![WarmItem {
            id: "7".to_string(),
            question: "List".to_string(),
            answer: "A, B".to_string(),
            note: None,
        }];
        let map = generate_format(&items, None, &ask_config(&cli), None).unwrap();
        let fmt = map.get("7").expect("a format for card 7");
        assert_eq!(fmt.back, ["A", "B"]);
        assert_eq!(fmt.mode, Some(Mode::LineByLine));
    }
}
