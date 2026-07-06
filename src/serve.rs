//! A local web frontend.
//!
//! `alix serve` starts a small synchronous HTTP server (one request at a
//! time — correct for a single user) that serves an embedded web page and a
//! JSON API. It is a third consumer of the same logic the TUI and browser use:
//! the [`Session`]/[`Store`] drive review, and cards are sent to the browser as
//! a DTO built from [`render::note_units`], so the note structuring lives in
//! one place. Grades persist to the same progress store, so studying in the
//! browser and on the command line share one history.
//!
//! It is deliberately local-only: no accounts, no database. By default it
//! binds to `127.0.0.1`; `--lan` binds all interfaces so a phone or tablet on
//! the same network can reach it (there is no authentication, so that is
//! opt-in).

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    hash::Hasher,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
    time::Instant,
};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server};
use twox_hash::XxHash64;

use crate::{
    answer::{Input, Mode, grade_lines_unordered, mode_name},
    ask::{self, CliSession, Exchange, Reply},
    augment::{self, AugmentCache},
    card::Card,
    choice,
    config::{
        AiConfig, AskConfig, Bindings, BrowseBindings, ExamConfig, Key, KeyPattern, PickerKeys,
        ReviewConfig, Strictness,
    },
    deck::{self, Deck, DeckState},
    exam, ladder, picker,
    recent::RecentDecks,
    render::{self, NoteUnit},
    scheduler::{Fsrs, Grade, keypoint_grade},
    session::{Session, now_ms},
    store::{self, Store},
    trace::{self, Delta, Excerpt, Phase, SourceBase, Walk},
};

/// Per-deck data the server needs to apply a removal: the file path, plus the
/// file's original text so removals can be re-derived from a fixed snapshot
/// (see [`deck::rewrite_without_cards`]).
struct DeckFiles {
    /// Subject → file path.
    paths: HashMap<String, PathBuf>,
    /// Subject → original file text (decks whose text could not be read are
    /// absent, and simply cannot have cards removed).
    snapshots: HashMap<String, String>,
    /// Subject → the 1-based front lines removed so far this run.
    removed: HashMap<String, BTreeSet<usize>>,
}

impl DeckFiles {
    fn new(paths: HashMap<String, PathBuf>) -> Self {
        let snapshots = paths
            .iter()
            .filter_map(|(subject, path)| {
                std::fs::read_to_string(path)
                    .ok()
                    .map(|text| (subject.clone(), text))
            })
            .collect();
        Self {
            paths,
            snapshots,
            removed: HashMap::new(),
        }
    }

    /// Appends condensed note lines to the card block at `line` of `subject`,
    /// then refreshes the snapshot so a later card removal keeps the new note
    /// (removals rewrite from the snapshot). Returns a message on failure.
    fn append_note(&mut self, subject: &str, line: usize, notes: &[String]) -> Result<(), String> {
        let path = self
            .paths
            .get(subject)
            .ok_or_else(|| format!("no deck file known for {subject}"))?;
        deck::append_note(path, line, notes).map_err(|e| e.to_string())?;
        if let Ok(text) = std::fs::read_to_string(path) {
            self.snapshots.insert(subject.to_string(), text);
        }
        Ok(())
    }

    /// Records that the card block at `line` of `subject` was removed and
    /// rewrites the deck file from its snapshot. Best-effort.
    fn remove_block(&mut self, subject: &str, line: usize) {
        let lines = self.removed.entry(subject.to_string()).or_default();
        lines.insert(line);
        if let (Some(path), Some(original)) = (self.paths.get(subject), self.snapshots.get(subject))
        {
            let lines: Vec<usize> = lines.iter().copied().collect();
            if let Err(e) = deck::rewrite_without_cards(path, original, &lines) {
                eprintln!("warning: could not update {}: {e}", path.display());
            }
        }
    }
}

const REVIEW_HTML: &str = include_str!("../assets/serve/review.html");
const THEME_CSS: &str = include_str!("../assets/serve/theme.css");
const THEME_JS: &str = include_str!("../assets/serve/theme.js");
const ALIX_LOGO_JS: &str = include_str!("../assets/serve/alix-logo.js");
const HEAD_HTML: &str = include_str!("../assets/serve/_head.html");
const BRAND_HTML: &str = include_str!("../assets/serve/_brand.html");

/// The review page with its shared-chrome placeholders filled once, so the head
/// boilerplate (`<!--%head%-->`) and brand mark (`<!--%brand%-->`) live in one place.
static REVIEW_PAGE: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| compose_page(REVIEW_HTML));

/// Fill the shared-chrome placeholders in a served page.
fn compose_page(html: &str) -> String {
    html.replace("<!--%head%-->", HEAD_HTML)
        .replace("<!--%brand%-->", BRAND_HTML)
}

/// One display unit of a card's note, ready for JSON. Mirrors
/// [`render::NoteUnit`]; the web page renders `sentence` as a paragraph and
/// `code` as a verbatim block.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum NoteUnitDto {
    Sentence { text: String },
    Code { lines: Vec<String> },
}

impl From<NoteUnit> for NoteUnitDto {
    fn from(unit: NoteUnit) -> Self {
        match unit {
            NoteUnit::Sentence(text) => NoteUnitDto::Sentence { text },
            NoteUnit::Code(lines) => NoteUnitDto::Code { lines },
        }
    }
}

/// A card serialized for the browser.
#[derive(Debug, Serialize)]
struct CardDto {
    front: String,
    context: Vec<String>,
    back: Vec<String>,
    /// True when `back` is a reshaped answer (a `format` augment's `display_back`),
    /// so the frontend bullets a multi-line list. Never set for the card's own
    /// authored back lines (a poem, typing answers) — only the reshape.
    reshaped: bool,
    note: Vec<NoteUnitDto>,
    /// `/img/<key>` URL for the question-side image, or `null`.
    img: Option<String>,
    /// `/img/<key>` URL for the answer-side image, shown on reveal, or `null`.
    img_back: Option<String>,
    /// The card's `% at:` source citation locator (e.g. `string.rs:120-128`),
    /// shown compact on reveal and expandable to `citation`. `null` if absent.
    at: Option<String>,
    /// The resolved excerpt for `at`, expanded in the browser on demand. `null`
    /// when the card has no `% at:` or it couldn't be resolved.
    citation: Option<ExcerptDto>,
    /// Why `at` couldn't be resolved (missing file, drifted line range), if it
    /// failed — shown dim in place of the excerpt.
    citation_error: Option<String>,
    /// The topological orientation breadcrumb — coarse region names with the
    /// current one marked — when the session is topology-ordered. `null`
    /// otherwise. Resolved by `review_state`, which holds the topology.
    crumb: Option<CrumbDto>,
}

/// The "where am I" region breadcrumb: the topology's region names in walk order
/// and the index of the current card's region. The page windows this to its
/// width (worst case: previous, current, next).
#[derive(Debug, Serialize)]
struct CrumbDto {
    regions: Vec<String>,
    current: usize,
    /// Per-region, per-card strength (`0..=1`, outer index aligns with
    /// `regions`) for the heatmap bar under each region — each card a cell,
    /// red (weak) → green (strong).
    cells: Vec<Vec<f32>>,
}

/// A deck's stored topologies with their region heatmaps, fetched on demand for
/// the picker's pre-launch **focus drawer** (`/api/deck-topology`): choose a
/// topology, see each region's strength, tap one to drill it.
#[derive(Debug, Serialize, Default)]
struct DeckTopologyDto {
    topologies: Vec<TopologyInfoDto>,
    /// Cards due/new across the whole deck right now — the count shown for the
    /// drawer's "Whole deck" option.
    deck_due: usize,
}

#[derive(Debug, Serialize)]
struct TopologyInfoDto {
    name: String,
    /// The one-line ordering principle (e.g. "north to south"), shown beside the
    /// name in the drawer's topology picker so several are told apart.
    principle: String,
    regions: Vec<RegionInfoDto>,
}

#[derive(Debug, Serialize)]
struct RegionInfoDto {
    name: String,
    /// Per-card strength (`0..=1`), red → green — the region's heatmap bar.
    cells: Vec<f32>,
    /// Cards due/new in this region right now — shown when it's the selection.
    due: usize,
}

/// The current review state sent to the browser after every action.
#[derive(Debug, Serialize)]
struct StateDto {
    /// Discriminates this payload from the trace-walk [`WalkDto`] for the single
    /// client dispatcher (`isWalk`): always `"review"` here.
    kind: &'static str,
    /// `"select"` while choosing decks (no session yet), `"review"` mid-session,
    /// `"done"` once nothing is left — the session-end signal. There is no
    /// separate `finished` flag; a finished session is just the `done` phase
    /// (matching the walk's own `done`).
    phase: &'static str,
    /// The card up for review, or `null` when finished or in the select phase.
    card: Option<CardDto>,
    /// For `choice` mode, the multiple-choice options (one is correct); `null`
    /// otherwise, or when the card has too few distractors (the page then
    /// falls back to reveal). The correct index is never sent here.
    choices: Option<Vec<String>>,
    /// For `explain` mode with cached key points, the rubric the reveal checks a
    /// reconstruction against — each ticked ✓/✗, the coverage deriving the grade.
    /// `null` for any other mode or when none are cached (plain self-grade).
    keypoints: Option<Vec<String>>,
    /// Whether the current card has never been seen and should be *acquired*
    /// (shown, then acknowledged with one key) rather than quizzed cold. The page
    /// shows the answer with a single "Seen" button and no grade controls.
    acquire: bool,
    /// The answer mode name (`flip`, `line`, …); the page reveals
    /// line-by-line for `line` and flip-style otherwise.
    mode: &'static str,
    /// The input method (`type` / `draw`). `draw` tells the page to show the
    /// canvas for a self-graded card; orthogonal to `mode`. The runtime "Draw
    /// answers" toggle lives in the browser and never appears here.
    input: &'static str,
    remaining: usize,
    initial: usize,
    reviews: usize,
    passed: usize,
    failed: usize,
    /// Per-stage counts (index 0 = unseen).
    histogram: [usize; 6],
    /// Subjects of decks in this (finished) session that are now `ExamDue` —
    /// drilled, sourced, and not yet mastered. The summary offers to sit each.
    /// Empty until the session is finished.
    exam_due: Vec<String>,
    /// Whether a restart would find any due/new cards right now. The summary
    /// disables "New session" and shows a "nothing due" note when this is
    /// false.
    can_restart: bool,
    /// Whether the current card is a virtual (remediation) card, so the page
    /// can offer to promote it into its deck file (appends it as a deck card,
    /// carrying over its review schedule rather than starting fresh). `false`
    /// once nothing is current, or for an authored deck card.
    promotable: bool,
    /// The highest reachable stage (always 5 now); the page would dim stages
    /// above it to `–`, but every deck reaches the top stage.
    top_stage: u8,
    label: String,
}

/// The payload of the browse view: every (remaining) card, in deck order, or
/// an empty list in the `select` phase.
#[derive(Debug, Serialize)]
struct BrowseDto {
    /// `"select"` while choosing decks, else `"browse"`.
    phase: &'static str,
    label: String,
    cards: Vec<CardDto>,
}

/// The deck-selection catalog sent to the browser picker, in the same three
/// sections as the TUI: `workspaces` (each with its last-progress time),
/// `recent` loose decks (recent-first), and plain `folders`. A deck inside a
/// workspace stays out of `recent` — you reach it by opening the workspace. The
/// filter searches every loose deck.
#[derive(Debug, Serialize)]
struct DeckListDto {
    workspaces: Vec<DeckItemDto>,
    recent: Vec<DeckItemDto>,
    folders: Vec<DeckItemDto>,
}

/// One row in the selection screen: its selection `name`, a display `label`, a
/// completion-state badge (`new` / `m/total` / `done ✓` / `mastered 🎉`), a
/// machine-readable `state` for styling, and the flags the picker dims/glyphs a
/// row with — `locked` (🔒, a `% requires:` prerequisite), `reviewable` (false →
/// 🕒, nothing due), `mastered` (🎉, tucked into the Mastered window). Mirrors
/// the TUI picker rows.
#[derive(Debug, Serialize)]
struct DeckItemDto {
    /// Stable selection key (file/folder name) sent back on select.
    name: String,
    /// Display title (`% title:`, else the name without `.txt`, else folder).
    label: String,
    meta: Option<String>,
    /// `new`/`started`/`finished`/`examdue` for a deck; `workspace`/`folder` for
    /// a drillable row.
    state: &'static str,
    locked: bool,
    /// Launching now would have something to do; `false` → nothing due (🕒).
    reviewable: bool,
    /// Finished *and* exam-passed — lives in the Mastered window, not Recent.
    mastered: bool,
    /// A trace deck (`% trace:`): walked, not card-reviewed.
    is_trace: bool,
    /// The AI exam can be sat now (has a `% source:`, not locked) — drilled or
    /// not, so the picker can offer "Take exam" to test out early.
    examable: bool,
    /// The deck *has* an exam at all (sourced, non-trace) even if it's locked —
    /// lets the footer always show a "Take exam" control, disabled when locked.
    has_exam: bool,
    /// `true` when this entry has recent-use history (shown in Recent by
    /// default; the rest are reachable through the filter).
    recent: bool,
    /// `true` for a workspace/folder row (opens into its members on click).
    is_workspace: bool,
    /// A workspace's one-line description (its learning goal), shown dim under
    /// the row; `null` for decks and folders.
    description: Option<String>,
    /// For a workspace/folder row: its member decks as an unlock dependency
    /// tree, shown when you open it.
    members: Vec<MemberDto>,
    /// Dim location hint (parent dir) for entries outside the decks dir; `null`
    /// keeps the row clean. Disambiguates same-named decks/workspaces.
    path: Option<String>,
    /// The `/img/<key>` URL of the workspace's icon, or `null` for the chevron.
    icon: Option<String>,
    /// `true` when `icon` is an SVG (rendered as a theme-tinted mask); a raster
    /// icon renders as a plain `<img>`.
    icon_svg: bool,
    /// `true` when the deck has a cached topology, so the picker's focus drawer
    /// would open for it — the row shows a small drawer indicator.
    has_topology: bool,
}

/// A workspace member deck in the drill-in list: a qualified selection `name`
/// (`<workspace>/<file>`), its display `label` and status (badge/state/locked/
/// reviewable/mastered/trace, from the workspace's own store), and its `depth`
/// in the unlock dependency tree (0 = a foundation root).
#[derive(Debug, Serialize)]
struct MemberDto {
    name: String,
    label: String,
    meta: Option<String>,
    state: &'static str,
    locked: bool,
    reviewable: bool,
    mastered: bool,
    is_trace: bool,
    examable: bool,
    has_exam: bool,
    depth: usize,
    /// The `├─`/`└─`/`│` tree-branch prefix drawn before the label, so the web
    /// shows the dependency tree like the TUI (not just indentation).
    tree: String,
    /// `true` when the member deck has a cached topology (a focus drawer).
    has_topology: bool,
}

/// The result of answering a choice card: which option was picked, which is
/// correct, and whether they match. The page highlights the options with this.
#[derive(Debug, Serialize)]
struct ChooseFeedbackDto {
    chosen: usize,
    correct: usize,
    passed: bool,
}

/// One typed line graded against the expected answer (typing / fuzzy mode).
#[derive(Debug, Serialize)]
struct LineResultDto {
    input: String,
    expected: String,
    passed: bool,
    distance: usize,
}

/// The result of submitting a typed answer: a result per back line plus whether
/// they all passed.
#[derive(Debug, Serialize)]
struct CheckFeedbackDto {
    results: Vec<LineResultDto>,
    passed: bool,
}

/// One ask-Claude exchange, for the browser.
#[derive(Debug, Serialize)]
struct ExchangeDto {
    q: String,
    a: String,
}

/// The ask-Claude view state: the conversation so far, whether a call is in
/// flight (the page polls while `thinking`), and a transient status / error.
#[derive(Debug, Serialize)]
struct AskDto {
    transcript: Vec<ExchangeDto>,
    thinking: bool,
    status: Option<String>,
    error: Option<String>,
}

/// The ask-tutor's model and effort, shown in the panel so it's clear which
/// model is answering — and that it's the CLI default (`"default"`) unless the
/// `[ask]` config pins one, not the stronger model that built the deck.
#[derive(Debug, Serialize)]
struct AskInfoDto {
    model: String,
    effort: String,
}

/// The running alix version, for the picker's About sheet.
#[derive(Serialize)]
struct VersionDto {
    version: &'static str,
}

impl AskInfoDto {
    fn from(cfg: &AskConfig) -> Self {
        let or_default = |s: &Option<String>| s.clone().unwrap_or_else(|| "default".to_string());
        Self {
            model: or_default(&cfg.model),
            effort: or_default(&cfg.effort),
        }
    }
}

/// One configured key, as the browser sees it: `k` is the `KeyboardEvent.key`
/// value (`" "`, `"Enter"`, `"j"`, …) and `ctrl` whether Ctrl must be held.
#[derive(Debug, Serialize)]
struct KeyDto {
    k: String,
    ctrl: bool,
}

fn key_dto(p: &KeyPattern) -> KeyDto {
    let k = match p.key {
        Key::Char(' ') => " ".to_string(),
        Key::Char(c) => c.to_string(),
        Key::Enter => "Enter".to_string(),
        Key::Tab => "Tab".to_string(),
        Key::Esc => "Escape".to_string(),
        Key::Backspace => "Backspace".to_string(),
    };
    KeyDto { k, ctrl: p.ctrl }
}

fn key_list(list: &[KeyPattern]) -> Vec<KeyDto> {
    list.iter().map(key_dto).collect()
}

/// The review actions the web page binds, mirroring the configured `[keys]`.
#[derive(Debug, Serialize)]
struct ReviewKeys {
    reveal: Vec<KeyDto>,
    failed: Vec<KeyDto>,
    partly: Vec<KeyDto>,
    passed: Vec<KeyDto>,
    skip: Vec<KeyDto>,
    remove: Vec<KeyDto>,
    restart: Vec<KeyDto>,
    ask: Vec<KeyDto>,
    save_note: Vec<KeyDto>,
}

impl ReviewKeys {
    fn from(b: &Bindings) -> Self {
        Self {
            reveal: key_list(&b.reveal),
            failed: key_list(&b.failed),
            partly: key_list(&b.partly),
            passed: key_list(&b.passed),
            skip: key_list(&b.skip),
            remove: key_list(&b.remove),
            restart: key_list(&b.restart),
            ask: key_list(&b.ask),
            save_note: key_list(&b.save_note),
        }
    }
}

/// The deck-picker navigation keys the selection screen binds, mirroring the
/// configured `[picker]` section (Vim defaults): move, open/back, focus the
/// filter, and open the Mastered window. (Jump to first/last is fixed at
/// `g`/`G`/Home/End on the page, so it isn't sent.)
#[derive(Debug, Serialize)]
struct PickerKeysDto {
    up: Vec<KeyDto>,
    down: Vec<KeyDto>,
    open: Vec<KeyDto>,
    back: Vec<KeyDto>,
    filter: Vec<KeyDto>,
    mastered: Vec<KeyDto>,
}

impl PickerKeysDto {
    fn from(k: &PickerKeys) -> Self {
        Self {
            up: key_list(&k.up),
            down: key_list(&k.down),
            open: key_list(&k.open),
            back: key_list(&k.back),
            filter: key_list(&k.filter),
            mastered: key_list(&k.mastered),
        }
    }
}

/// The browse actions the web page binds, mirroring the configured `[browse]`.
#[derive(Debug, Serialize)]
struct BrowseKeys {
    next: Vec<KeyDto>,
    prev: Vec<KeyDto>,
    remove: Vec<KeyDto>,
}

impl BrowseKeys {
    fn from(b: &BrowseBindings) -> Self {
        Self {
            next: key_list(&b.next),
            prev: key_list(&b.prev),
            remove: key_list(&b.remove),
        }
    }
}

/// Global options for a served review, independent of which decks are chosen
/// (the per-session label and deck paths come from [`SessionBuild`]).
pub struct ReviewOptions {
    pub keys: Bindings,
    /// Deck-picker navigation keys (the `[picker]` section), bound on the
    /// selection screen.
    pub picker: PickerKeys,
    /// Browse-mode keys (the `[browse]` section), bound on the `/browse` page
    /// this server also hosts.
    pub browse: BrowseBindings,
    /// Fuzzy-mode typo tolerance per line.
    pub max_typos: usize,
    /// Ask-Claude settings (command, allowlist, timeout, …).
    pub ask: AskConfig,
    /// AI-exam settings (model, question count, default strictness, …).
    pub exam: ExamConfig,
    /// AI augmentation settings (model, per-target counts), for generating
    /// augmentations from the picker's Augment screen.
    pub ai: AiConfig,
    /// Personal review pacing (FSRS retention + retirement interval), for the
    /// selection screen's badges and due counts.
    pub review: ReviewConfig,
    /// Pairing token required on `/api/*` when set (auto-generated for `--lan`);
    /// `None` leaves the server open (the localhost default).
    pub auth: Option<String>,
}

/// A review session ready to serve: the session, its header label, the
/// subject → deck file path map used for card removal, and the subject → deck
/// reference links (`% link:`) offered to ask-Claude. Produced by the caller's
/// builder closure when decks are chosen (on the CLI or in the browser picker).
pub struct SessionBuild {
    pub session: Session,
    pub label: String,
    pub decks: HashMap<String, PathBuf>,
    pub links: HashMap<String, Vec<String>>,
    /// Subject → its deck's `% source:` project root, for the grounded ask-tutor
    /// (`[ask] source_access`). Only decks with a local source appear.
    pub source_roots: HashMap<String, PathBuf>,
    /// Subject → its deck's source base, for resolving a card's `% at:` citation
    /// excerpt on reveal.
    pub source_bases: HashMap<String, SourceBase>,
    /// The resolved topology name when this session is topology-ordered, so the
    /// server can show the connective cue from that topology. `None` otherwise.
    pub topology_name: Option<String>,
}

/// A trace walk ready to serve, built when a single trace deck is picked from the
/// review server's deck-selection screen. The walk is self-graded (no live
/// `--grade`), matching the terminal picker's trace → walk.
pub struct WalkBuild {
    pub walk: Walk,
}

/// A browse card list ready to serve, with its label and deck paths.
pub struct CardsBuild {
    pub cards: Vec<Card>,
    pub label: String,
    pub decks: HashMap<String, PathBuf>,
}

/// What the review server opens on: a live review session, a read-only browse
/// list, or — neither — the in-browser deck picker. Browse is a native mode of
/// the review server (no separate page), so `alix browse --serve` seeds `Browse`
/// exactly as review seeds `Review`.
pub enum Launch {
    Review(Box<SessionBuild>),
    Browse(CardsBuild),
    Picker,
}

/// The server's live review state once decks are chosen. Its absence (`None`)
/// means the page is in the deck-selection phase.
struct Reviewing {
    session: Session,
    label: String,
    files: DeckFiles,
    images: HashMap<String, PathBuf>,
    /// Ask-Claude tutor (the CLI conversation, transcript and in-flight call),
    /// shared with the trace walk; the per-subject `% link:` links and source
    /// roots that ground it stay here (they're keyed by deck subject).
    ask: Ask,
    links: HashMap<String, Vec<String>>,
    /// Subject → `% source:` project root, for the grounded tutor (opt-in).
    source_roots: HashMap<String, PathBuf>,
    /// Subject → source base, for resolving a card's `% at:` citation excerpt.
    source_bases: HashMap<String, SourceBase>,
    /// AI distractors for choice cards, read when building a choice question
    /// (generated ahead of time by `alix deck augment`; empty → offline).
    augment: AugmentCache,
    /// A per-presentation counter (seeded from the clock) that rotates a reworded
    /// question variant in each time a card is shown (`--target questions`).
    present_seq: u64,
    /// The authored front of each card we've rotated a variant into, so the
    /// original phrasing stays in the rotation (generated variants drop it).
    original_fronts: HashMap<u64, String>,
    /// The resolved topology name when this session is topology-ordered (used to
    /// fetch the topology from `augment` for the orientation breadcrumb); `None`
    /// otherwise.
    topology_name: Option<String>,
}

/// An in-flight ask-Claude call: the channel the background thread answers on,
/// what it is for, and the card it is about (snapshotted so a late reply still
/// refers to the right card).
struct Pending {
    rx: Receiver<Reply>,
    purpose: Purpose,
    card: Card,
}

/// What a pending CLI call will do with its answer.
enum Purpose {
    /// A question; holds the text to record in the transcript on success.
    Question(String),
    /// Condense the conversation into note lines appended to the deck file.
    Condense,
}

/// The ask-Claude tutor's state, shared by a review session and a trace walk: the
/// CLI conversation spanning the session, the running transcript shown for the
/// current subject (a review card or a walk checkpoint), and any in-flight call.
/// It is agnostic to *what* is being studied — the consumer supplies the subject
/// [`Card`], its `% link:` links and source root per call, and (on a "save note"
/// condense) writes the resulting note where the subject lives.
struct Ask {
    cli: CliSession,
    transcript: Vec<Exchange>,
    /// The subject id the displayed transcript belongs to; cleared when the
    /// subject changes (the CLI conversation itself still spans the whole
    /// session, so Claude keeps the full context).
    subject: Option<u64>,
    pending: Option<Pending>,
}

impl Ask {
    fn new() -> Self {
        Self {
            cli: CliSession::new(),
            transcript: Vec::new(),
            subject: None,
            pending: None,
        }
    }

    /// Drops the displayed transcript when the subject (card/checkpoint) changes,
    /// so the ask view shows only the current subject's exchanges.
    fn align(&mut self, subject: Option<u64>) {
        if self.subject != subject {
            self.transcript.clear();
            self.subject = subject;
        }
    }

    fn dto(&self, status: Option<String>, error: Option<String>) -> AskDto {
        AskDto {
            transcript: self
                .transcript
                .iter()
                .map(|(q, a)| ExchangeDto {
                    q: q.clone(),
                    a: a.clone(),
                })
                .collect(),
            thinking: self.pending.is_some(),
            status,
            error,
        }
    }

    /// Starts a question about `card` (or, with `question: None`, condenses the
    /// conversation into note lines). `links`/`root` ground the tutor exactly as a
    /// review does. Returns `false` (no-op) if a call is already pending or a
    /// condense has nothing to summarize.
    fn start(
        &mut self,
        cfg: &AskConfig,
        card: &Card,
        links: &[String],
        root: Option<&Path>,
        frozen: Option<&str>,
        question: Option<String>,
    ) -> bool {
        if self.pending.is_some() {
            return false;
        }
        // A new subject starts a fresh visible transcript (and a subject-scoped
        // condense), even though the CLI conversation continues.
        self.align(Some(card.id()));
        if question.is_none() && self.transcript.is_empty() {
            return false;
        }
        let run_cfg = match root {
            Some(r) => ask::with_source_root(cfg, r),
            None => cfg.clone(),
        };
        // Reconcile the session with this call's cwd *before* building the prompt:
        // a cwd change starts a fresh conversation, so `started` then reports this
        // as a first message (full subject context).
        let args = self.cli.args_in(run_cfg.cwd.as_deref());
        // Claude keeps the running conversation via `--resume`, so its follow-up
        // prompt stays short. A backend without a session (Task 7) runs each turn
        // statelessly, so re-inline the prior transcript to restore memory.
        let keeps_session = crate::backend::backend_for(&run_cfg)
            .map(|b| b.supports_session())
            .unwrap_or(false);
        let (prompt, purpose) = match question {
            Some(q) => {
                let prompt = if keeps_session {
                    ask::question_prompt(card, links, &q, !self.cli.started, root, frozen)
                } else {
                    ask::question_prompt_with_history(
                        card,
                        links,
                        &self.transcript,
                        &q,
                        root,
                        frozen,
                    )
                };
                (prompt, Purpose::Question(q))
            }
            None => (
                ask::condense_prompt(card, &self.transcript),
                Purpose::Condense,
            ),
        };
        let rx = ask::spawn(run_cfg, prompt, args);
        self.pending = Some(Pending {
            rx,
            purpose,
            card: card.clone(),
        });
        true
    }

    /// Drains a finished reply: a question lands in the transcript; a condense's
    /// note lines are handed to `save` (the consumer writes them where the subject
    /// lives — a deck card, or a trace checkpoint). Returns a one-shot
    /// `(status, error)` to show once.
    fn poll(
        &mut self,
        save: impl FnOnce(&Card, &[String]) -> Result<(), String>,
    ) -> (Option<String>, Option<String>) {
        let reply = match &self.pending {
            None => return (None, None),
            Some(p) => match p.rx.try_recv() {
                Ok(reply) => reply,
                Err(TryRecvError::Empty) => return (None, None),
                Err(TryRecvError::Disconnected) => {
                    Reply::Error("the ask helper exited unexpectedly".to_string())
                }
            },
        };
        let pending = self.pending.take().expect("pending was present");
        match (reply, pending.purpose) {
            (Reply::Answer(answer), Purpose::Question(question)) => {
                self.cli.started = true; // later calls --resume this conversation
                self.transcript.push((question, answer));
                (None, None)
            }
            (Reply::Answer(text), Purpose::Condense) => {
                self.cli.started = true;
                let notes = ask::extract_note_lines(&text);
                if notes.is_empty() {
                    return (Some("nothing to save".to_string()), None);
                }
                match save(&pending.card, &notes) {
                    Ok(()) => (Some("note saved".to_string()), None),
                    Err(e) => (None, Some(e)),
                }
            }
            // Don't resume a session in an unknown state; the next question starts
            // a fresh one.
            (Reply::Error(e), _) => {
                self.cli = CliSession::new();
                (None, Some(e))
            }
        }
    }
}

impl Reviewing {
    fn new(build: SessionBuild) -> Self {
        let images = collect_images(build.session.cards());
        Self {
            session: build.session,
            label: build.label,
            files: DeckFiles::new(build.decks),
            images,
            ask: Ask::new(),
            links: build.links,
            source_roots: build.source_roots,
            source_bases: build.source_bases,
            // The real cache is opened by `open_augment` once the active store
            // path is known; until then an empty cache (offline only).
            augment: AugmentCache::open(Path::new("")),
            present_seq: now_ms(),
            original_fronts: HashMap::new(),
            topology_name: build.topology_name,
        }
    }

    /// Opens the distractor cache co-located with the active `store_path` (the
    /// active store changes per selection). Distractors are generated ahead of
    /// time by `alix deck augment`; review only reads them.
    fn open_augment(&mut self, store_path: &Path) {
        self.augment = AugmentCache::open(augment::augment_path_for(store_path));
    }

    /// Rotates the current card's question through the pool of its authored front
    /// plus any cached variants (`alix deck augment --target questions`), a fresh
    /// phrasing each time a card is presented. The answer is unchanged, so
    /// identity (which ignores the front) is untouched. Called on card advance.
    fn rotate_variant(&mut self) {
        let Some(id) = self.session.current().map(|c| c.id()) else {
            return;
        };
        if self.augment.variants(id).is_none() {
            return;
        }
        // Capture the authored front the first time, before we overwrite it, so
        // it stays in the rotation alongside the generated variants.
        if !self.original_fronts.contains_key(&id)
            && let Some(card) = self.session.current()
        {
            self.original_fronts.insert(id, card.front.clone());
        }
        let original = self.original_fronts.get(&id).cloned().unwrap_or_default();
        let seed = self.present_seq;
        self.present_seq = self.present_seq.wrapping_add(1);
        if let Some(chosen) = self.augment.pick_front(id, &original, seed)
            && let Some(card) = self.session.current_mut()
        {
            card.front = chosen;
        }
    }

    /// Drops the displayed transcript when the current card changed, so the ask
    /// view shows only this card's exchanges. The CLI session (`cli`) is
    /// untouched, so Claude still has the whole conversation as context.
    fn align_transcript(&mut self) {
        self.ask.align(self.session.current().map(|c| c.id()));
    }

    /// The ask-view payload, with an optional one-shot status/error.
    fn ask_dto(&self, status: Option<String>, error: Option<String>) -> AskDto {
        self.ask.dto(status, error)
    }

    /// Starts an ask-Claude call about the current card. `question` is the text
    /// to ask; `None` condenses the conversation into deck notes instead. Returns
    /// `false` (no-op) if a call is already pending, nothing is reviewable, or
    /// there is nothing to condense. Grounds the tutor in the card's deck source
    /// when that deck opted into `[ask] source_access` (`source_roots`).
    fn start_ask(&mut self, cfg: &AskConfig, question: Option<String>) -> bool {
        let Some(card) = self.session.current().cloned() else {
            return false;
        };
        let links = self.links.get(&*card.subject).cloned().unwrap_or_default();
        // The grounded source root (opt-in via `source_access`): a per-card
        // `% origin:` override, else the deck/workspace root.
        let root = self.source_roots.get(&*card.subject).map(|deck_root| {
            card.origin
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| deck_root.clone())
        });
        // A frozen card inlines its snapshot excerpt as the anchor; the live
        // source is read for context. A recorded-but-missing source → the canned
        // "couldn't find" reply (no cwd handed to the subprocess).
        let frozen = root.as_ref().and_then(|_| {
            let at = card.at.as_deref()?;
            let base = self.source_bases.get(&*card.subject)?;
            trace::frozen_excerpt_block(at, card.at_origin.as_deref(), base)
        });
        let live_root = root.as_deref().filter(|r| r.exists());
        self.ask
            .start(cfg, &card, &links, live_root, frozen.as_deref(), question)
    }

    /// Drains a finished CLI reply into the transcript (a question) or the deck
    /// file (a "save note" condense). Returns a transient `(status, error)`.
    fn poll_ask(&mut self) -> (Option<String>, Option<String>) {
        // Opening the ask view on a new card (the page polls `/api/ask`) drops
        // the previous card's exchanges from the display.
        self.align_transcript();
        // Field-split the borrow so the save closure can touch `files`/`session`
        // while `ask` drives the poll.
        let Self {
            ask,
            files,
            session,
            ..
        } = self;
        ask.poll(|card, notes| {
            files.append_note(&card.subject, card.line, notes)?;
            // Mirror the note onto the in-memory card so returning to it shows the
            // note at once, without re-reading the deck.
            if let Some(cur) = session.current_mut()
                && cur.id() == card.id()
            {
                cur.append_note(notes);
            }
            Ok(())
        })
    }
}

/// The server's live AI-exam state: one in-progress [`exam::Sitting`] plus the
/// path of the deck under exam (to resolve what a pass unlocks).
struct Examining {
    sitting: exam::Sitting,
    deck_path: PathBuf,
}

/// The exam payload sent to the browser. The page renders sub-views off `phase`
/// and polls `GET /api/exam` while `thinking`.
#[derive(Serialize)]
struct ExamDto {
    phase: &'static str,
    deck: String,
    strictness: &'static str,
    total: usize,
    current: usize,
    /// The current question's prompt (in the answering phase).
    question: Option<String>,
    /// The answer saved for the current question so far.
    answer: String,
    on_last: bool,
    /// Per-question breakdown (results phase).
    grades: Vec<ExamGradeDto>,
    passed: Option<bool>,
    gaps: Vec<String>,
    /// Whether a failed result can be remediated into cards (fact decks only — a
    /// trace is re-walked, not remediated). Drives the remediation button.
    can_remediate: bool,
    /// How many remediation cards the last remediation created or revived;
    /// `None` until one completes (the "remediated" phase renders it).
    remediated_count: Option<usize>,
    /// A trace exam (the compression), so the page shows a "re-walk" hint on a
    /// fail instead of remediation.
    is_trace: bool,
    /// Decks a pass unlocks.
    unlocks: Vec<String>,
    thinking: bool,
    error: Option<String>,
    /// Seconds the in-flight Claude call has been running (progress feedback).
    elapsed: Option<u64>,
}

#[derive(Serialize)]
struct ExamGradeDto {
    question: String,
    points: Vec<String>,
    answer: String,
    verdict: &'static str,
    feedback: String,
    missed: Vec<String>,
}

fn exam_phase_name(phase: &exam::Phase) -> &'static str {
    match phase {
        exam::Phase::Generating => "generating",
        exam::Phase::Answering => "answering",
        exam::Phase::Grading => "grading",
        exam::Phase::Results => "results",
        exam::Phase::Remediating => "remediating",
        exam::Phase::Remediated => "remediated",
    }
}

fn strictness_name(s: Strictness) -> &'static str {
    match s {
        Strictness::Strict => "strict",
        Strictness::Balanced => "balanced",
        Strictness::Lenient => "lenient",
    }
}

/// Serializes an exam sitting for the browser; `decks_dir` resolves the
/// dependent decks shown on a pass.
fn exam_dto(ex: &Examining, decks_dir: &Path) -> ExamDto {
    let s = &ex.sitting;
    let result = s.result();
    let grades = result
        .map(|r| {
            s.questions()
                .iter()
                .zip(s.answers())
                .zip(&r.grades)
                .map(|((q, a), g)| ExamGradeDto {
                    question: q.prompt.clone(),
                    points: q.points.clone(),
                    answer: a.clone(),
                    verdict: g.verdict.label(),
                    feedback: g.feedback.clone(),
                    missed: g.missed.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let passed = result.map(|r| r.passed);
    let unlocks = if passed == Some(true) {
        deck::dependents(&ex.deck_path, decks_dir)
    } else {
        Vec::new()
    };
    ExamDto {
        phase: exam_phase_name(s.phase()),
        deck: s.subject().to_string(),
        strictness: strictness_name(s.strictness()),
        total: s.total(),
        current: s.current_index(),
        question: s.question().map(|q| q.prompt.clone()),
        answer: s.answer().to_string(),
        on_last: s.on_last(),
        grades,
        passed,
        gaps: s.gaps(),
        can_remediate: s.can_remediate(),
        remediated_count: s.remediated_count(),
        is_trace: s.kind() == exam::SittingKind::Trace,
        unlocks,
        thinking: s.thinking(),
        error: s.error().map(str::to_string),
        elapsed: s.elapsed_secs(),
    }
}

/// The server's live deck-augmentation state: one deck's augmentation cache and
/// any in-flight generation. Opened from the picker's Augment screen
/// (`/api/augment/open`), it reports coverage, fills gaps, and removes — all
/// scoped to this deck, since the cache may be shared by other decks on the same
/// store. The single in-flight `Job` runs on a background thread (`augment::spawn`)
/// while the page polls `GET /api/augment`.
struct Augmenting {
    /// Display name (a workspace member's qualified `<ws>/<file>`, or a deck file).
    deck: String,
    cards: Vec<Card>,
    /// This deck's card ids, for scoping removals against the shared cache.
    deck_ids: HashSet<u64>,
    cache: AugmentCache,
    pending: Option<AugmentPending>,
    /// The last generation/save error, shown until the next action clears it.
    error: Option<String>,
}

/// An augmentation generation in flight: the channel the worker delivers on, the
/// target it's filling (for the "busy" row), and when it started (for elapsed).
struct AugmentPending {
    rx: Receiver<Result<augment::Outcome, String>>,
    target: &'static str,
    started: Instant,
}

/// The Augment screen payload. `rows` is a flat, data-driven list so a new
/// augmentation target is one more row with no page-layout change.
#[derive(Serialize)]
struct AugmentDto {
    deck: String,
    cards: usize,
    rows: Vec<AugmentRowDto>,
    /// The target currently generating, if any (the page shows a spinner + polls).
    busy: Option<&'static str>,
    /// Seconds the in-flight generation has run (progress feedback).
    elapsed: Option<u64>,
    error: Option<String>,
}

/// One target's row. Per-card targets carry `covered`/`eligible`; the topology
/// row carries its named `items` instead (the page branches on `kind`).
#[derive(Serialize)]
struct AugmentRowDto {
    kind: &'static str,
    label: &'static str,
    covered: usize,
    eligible: usize,
    items: Vec<String>,
    busy: bool,
}

impl Augmenting {
    /// Opens the Augment screen for a deck: loads its augmentation cache (beside
    /// the deck's store) and records its cards for coverage + gap computation.
    fn open(deck: String, cards: Vec<Card>, cache_path: PathBuf) -> Self {
        let deck_ids = cards.iter().map(Card::id).collect();
        Self {
            deck,
            cards,
            deck_ids,
            cache: AugmentCache::open(cache_path),
            pending: None,
            error: None,
        }
    }

    /// Builds the screen payload from the current cache coverage + any in-flight job.
    fn dto(&self) -> AugmentDto {
        let s = self.cache.summarize(&self.cards);
        let busy = self.pending.as_ref().map(|p| p.target);
        let card_row =
            |kind: &'static str, label: &'static str, c: augment::Coverage| AugmentRowDto {
                kind,
                label,
                covered: c.covered,
                eligible: c.eligible,
                items: Vec::new(),
                busy: busy == Some(kind),
            };
        let rows = vec![
            card_row("choices", "Choices", s.choices),
            card_row("notes", "Notes", s.notes),
            card_row("questions", "Questions", s.questions),
            card_row("keypoints", "Key points", s.keypoints),
            card_row("format", "Formatting", s.format),
            AugmentRowDto {
                kind: "topology",
                label: "Topology",
                covered: 0,
                eligible: 0,
                items: s.topologies,
                busy: busy == Some("topology"),
            },
        ];
        AugmentDto {
            deck: self.deck.clone(),
            cards: self.cards.len(),
            rows,
            busy,
            elapsed: self.pending.as_ref().map(|p| p.started.elapsed().as_secs()),
            error: self.error.clone(),
        }
    }

    /// Spawns fill-the-gaps generation for `target` (no-op if one is already
    /// running, the target is unknown, or a per-card target has no gap to fill).
    /// `guidance` is the `--with` steer. Returns whether a job started.
    fn generate(
        &mut self,
        target: &str,
        guidance: Option<String>,
        ai: &AiConfig,
        ask: &AskConfig,
    ) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let (job, tgt): (augment::Job, &'static str) = match target {
            "choices" => {
                let items = self.cache.missing_choices(&self.cards);
                if items.is_empty() {
                    return false;
                }
                (
                    augment::Job::Choices {
                        items,
                        count: ai.distractor_count,
                    },
                    "choices",
                )
            }
            "notes" => {
                let items = self.cache.missing_notes(&self.cards);
                if items.is_empty() {
                    return false;
                }
                (augment::Job::Notes { items }, "notes")
            }
            "questions" => {
                let items = self.cache.missing_questions(&self.cards);
                if items.is_empty() {
                    return false;
                }
                (
                    augment::Job::Questions {
                        items,
                        count: ai.variant_count,
                    },
                    "questions",
                )
            }
            "keypoints" => {
                let items = self.cache.missing_keypoints(&self.cards);
                if items.is_empty() {
                    return false;
                }
                (
                    augment::Job::Keypoints {
                        items,
                        count: ai.keypoint_count,
                    },
                    "keypoints",
                )
            }
            "format" => {
                let items = self.cache.missing_format(&self.cards);
                if items.is_empty() {
                    return false;
                }
                (augment::Job::Format { items }, "format")
            }
            // Topology always adds a new one (named by its guidance); no gap notion.
            "topology" => (
                augment::Job::Topology {
                    items: self
                        .cards
                        .iter()
                        .map(augment::WarmItem::from_card)
                        .collect(),
                },
                "topology",
            ),
            _ => return false,
        };
        self.error = None;
        let rx = augment::spawn(job, guidance, augment::run_config(ai, ask));
        self.pending = Some(AugmentPending {
            rx,
            target: tgt,
            started: Instant::now(),
        });
        true
    }

    /// Drains a finished generation: applies its [`Outcome`](augment::Outcome) to
    /// the cache and saves, or records the error. A no-op while still running.
    fn poll(&mut self) {
        let Some(p) = self.pending.as_ref() else {
            return;
        };
        let outcome = match p.rx.try_recv() {
            Ok(reply) => reply,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("the augment helper exited unexpectedly".to_string())
            }
        };
        self.pending = None;
        match outcome {
            Ok(o) => {
                self.apply(o);
                self.save();
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Writes a finished outcome into the cache (does not save).
    fn apply(&mut self, outcome: augment::Outcome) {
        match outcome {
            augment::Outcome::Choices(map) => {
                for (id, v) in map {
                    self.cache.set_distractors(id, v);
                }
            }
            augment::Outcome::Notes(map) => {
                for (id, v) in map {
                    self.cache.set_note(id, v);
                }
            }
            augment::Outcome::Questions(map) => {
                for (id, v) in map {
                    self.cache.set_variants(id, v);
                }
            }
            augment::Outcome::Keypoints(map) => {
                for (id, v) in map {
                    self.cache.set_keypoints(id, v);
                }
            }
            augment::Outcome::Topology(t) => self.cache.add_topology(t),
            augment::Outcome::Format(map) => {
                for (id, v) in map {
                    self.cache.set_format(id, v);
                }
            }
        }
    }

    /// Removes a target's augmentations for this deck, then saves. `topology`
    /// names the one to drop when `target` is `"topology"`; `"all"` clears
    /// everything this deck owns. Returns whether the request was understood.
    fn remove(&mut self, target: &str, topology: Option<&str>) -> bool {
        match target {
            "choices" => self.cache.clear_distractors(&self.deck_ids),
            "notes" => self.cache.clear_notes(&self.deck_ids),
            "questions" => self.cache.clear_variants(&self.deck_ids),
            "keypoints" => self.cache.clear_keypoints(&self.deck_ids),
            "format" => self.cache.clear_format(&self.deck_ids),
            "topology" => {
                let Some(name) = topology else {
                    return false;
                };
                self.cache.remove_topology(name, &self.deck_ids);
            }
            "all" => self.cache.clear_all(&self.deck_ids),
            _ => return false,
        }
        self.error = None;
        self.save();
        true
    }

    /// Persists the cache, recording any I/O error for the page to surface.
    fn save(&mut self) {
        if let Err(e) = self.cache.save() {
            self.error = Some(format!("could not save augmentations: {e}"));
        }
    }
}

/// Serves review at `addr` until the process is stopped. `initial` decides what
/// it opens on: a review session, a read-only browse list, or (`Launch::Picker`)
/// the in-browser deck-selection screen; picking decks (`POST /api/select`)
/// calls `build` to construct a session in place.
/// `build` borrows the shared `store` and `recent`, so all sessions write one
/// history and update the recent-decks list, exactly like the CLI.
#[expect(clippy::too_many_arguments)] // each is a distinct, named server input
pub fn run_review(
    initial: Launch,
    mut store: Store,
    mut recent: RecentDecks,
    decks_dir: PathBuf,
    addr: SocketAddr,
    opts: ReviewOptions,
    mut build: impl FnMut(
        Vec<PathBuf>,
        Option<&str>,
        Option<&str>,
        &Store,
        &mut RecentDecks,
    ) -> Result<SessionBuild>,
    // Builds a walk when the picked decks are a single trace (else `None`, so the
    // caller flattens to a review); mirrors the terminal picker's trace → walk.
    mut build_walk: impl FnMut(&[PathBuf]) -> Result<Option<WalkBuild>>,
    // Builds a read-only browse card list from the picked decks (the picker's
    // "Browse" action; the page navigates to `/browse`, which this server hosts).
    mut build_browse: impl FnMut(Vec<PathBuf>, &mut RecentDecks) -> Result<CardsBuild>,
    // The progress store the given decks write to — a workspace's own
    // `progress.json` when they share one, else the global store (`&[]` → global),
    // mirroring the terminal `store_for`. The active store is swapped to this when
    // a session launches and reset to the global one back at the picker.
    mut store_for: impl FnMut(&[PathBuf]) -> Result<Store>,
) -> Result<()> {
    let ReviewOptions {
        keys: bindings,
        picker: picker_keys,
        browse: browse_bindings,
        max_typos,
        ask: ask_cfg,
        exam: exam_cfg,
        ai: ai_cfg,
        review: review_cfg,
        auth,
    } = opts;
    let keys = ReviewKeys::from(&bindings);
    let picker_keys = PickerKeysDto::from(&picker_keys);
    // The `/browse` page this server also hosts needs its own next/prev/remove
    // keys, distinct from the review grade keys served at `/api/keys`.
    let browse_keys = BrowseKeys::from(&browse_bindings);
    let ask_info = AskInfoDto::from(&ask_cfg);
    // Browse is a native mode of the review server: `initial` seeds either a
    // review session, a read-only browse list, or neither (the picker).
    let (mut reviewing, mut browsing) = match initial {
        Launch::Review(b) => (Some(Reviewing::new(*b)), None),
        Launch::Browse(b) => (None, Some(Browsing::new(b))),
        Launch::Picker => (None, None),
    };
    if let Some(r) = reviewing.as_mut() {
        r.open_augment(store.path());
        r.rotate_variant();
    }
    let mut examining: Option<Examining> = None;
    // The picker's "Augment" action opens a deck's augmentation screen here.
    let mut augmenting: Option<Augmenting> = None;
    // A trace picked from the selection screen walks in-page inside review.html
    // (no navigation to a separate `/walk` page — the walk is an in-page mode).
    let mut walking: Option<Walking> = None;
    // `browsing` (seeded above for a `--serve` browse launch) is also entered
    // from the picker's "Browse" action (POST /api/browse) — in-page, no page nav.
    // Workspace icons resolved while building the picker, served via `/img/` at
    // launcher time (when no review/browse session owns the registry).
    let mut launcher_icons: HashMap<String, PathBuf> = HashMap::new();
    let server = Server::http(addr).map_err(|e| anyhow!("cannot start server on {addr}: {e}"))?;
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let path = request_path(&request);
        if !is_authorized(
            &path,
            header_value(&request, "Authorization"),
            query_param(request.url(), "token").as_deref(),
            auth.as_deref(),
        ) {
            respond_status(request, 401);
            continue;
        }
        match (&method, path.as_str()) {
            (Method::Get, "/") => respond_html(request, &REVIEW_PAGE),
            (Method::Get, "/theme.css") => {
                respond_asset(request, THEME_CSS, "text/css; charset=utf-8")
            }
            (Method::Get, "/theme.js") => {
                respond_asset(request, THEME_JS, "application/javascript; charset=utf-8")
            }
            (Method::Get, "/alix-logo.js") => respond_asset(
                request,
                ALIX_LOGO_JS,
                "application/javascript; charset=utf-8",
            ),
            (Method::Get, "/api/keys") => respond_json(request, &keys),
            (Method::Get, "/api/version") => respond_json(
                request,
                &VersionDto {
                    version: env!("CARGO_PKG_VERSION"),
                },
            ),
            (Method::Get, "/api/browse-keys") => respond_json(request, &browse_keys),
            (Method::Get, "/api/picker-keys") => respond_json(request, &picker_keys),
            (Method::Get, "/api/ask-info") => respond_json(request, &ask_info),
            (Method::Get, "/api/decks") => {
                // Review enforces locking; the picker won't start a locked deck.
                let catalog = deck_catalog(
                    &decks_dir,
                    &recent,
                    &store,
                    true,
                    &mut launcher_icons,
                    review_cfg,
                );
                respond_json(request, &catalog)
            }
            // Image cards: served from whichever session is live (review or browse).
            (Method::Get, key) if key.starts_with("/img/") => {
                let name = &key["/img/".len()..];
                if let Some(r) = &reviewing {
                    serve_image(request, &r.images, name)
                } else if let Some(b) = &browsing {
                    serve_image(request, &b.images, name)
                } else {
                    serve_image(request, &launcher_icons, name)
                }
            }
            (Method::Get, "/api/state") => {
                // Browse is an in-page mode: when a browse list is live (a
                // `--serve` browse launch), the page gets the browse payload here
                // and opens the browse overlay instead of a review session.
                if let Some(b) = &browsing {
                    respond_json(request, &browse_payload(Some(b)))
                } else {
                    // A missed card may have cooled back into due-ness since the
                    // last fetch; re-check so it re-enters review on this poll
                    // (stats preserved), no manual restart needed.
                    if let Some(r) = reviewing.as_mut() {
                        r.session.poll(&store, now_ms());
                    }
                    respond_json(
                        request,
                        &review_state(reviewing.as_ref(), &store),
                    )
                }
            }
            (Method::Post, "/api/select") => {
                match read_selection(&mut request, &decks_dir, &recent) {
                    Some(sel) => {
                        let (topology, region) = (sel.topology.clone(), sel.region.clone());
                        let paths = vec![sel.deck];
                        // Write to the deck's own store — a workspace's `progress.json`
                        // when they share one, else the global store — so the browser
                        // records progress where the TUI would and where the picker's
                        // badges are read from.
                        if let Err(e) = store_for(&paths).map(|s| store = s) {
                            eprintln!("warning: could not open the progress store: {e}");
                            respond_status(request, 400);
                            continue;
                        }
                        match build_walk(&paths) {
                            Ok(Some(wb)) => {
                                let w = Walking::new(wb.walk, None);
                                let dto = walk_dto(&w);
                                walking = Some(w);
                                reviewing = None;
                                examining = None;
                                respond_json(request, &dto);
                            }
                            Ok(None) => match build(
                                paths,
                                topology.as_deref(),
                                region.as_deref(),
                                &store,
                                &mut recent,
                            ) {
                                Ok(b) => {
                                    let mut r = Reviewing::new(b);
                                    r.open_augment(store.path());
                                    r.rotate_variant();
                                    reviewing = Some(r);
                                    walking = None;
                                    respond_json(
                                        request,
                                        &review_state(reviewing.as_ref(), &store),
                                    );
                                }
                                Err(e) => {
                                    eprintln!("warning: could not load the selected decks: {e}");
                                    respond_status(request, 400);
                                }
                            },
                            Err(e) => {
                                eprintln!("warning: could not load the selected trace: {e}");
                                respond_status(request, 400);
                            }
                        }
                    }
                    None => respond_status(request, 400),
                }
            }
            // The picker's "Browse" action: build a read-only card list and return
            // it, so the page opens the browse overlay in place (no page nav).
            (Method::Post, "/api/browse") => {
                match read_selection(&mut request, &decks_dir, &recent) {
                    Some(sel) => {
                        let paths = vec![sel.deck];
                        if let Err(e) = store_for(&paths).map(|s| store = s) {
                            eprintln!("warning: could not open the progress store: {e}");
                            respond_status(request, 400);
                            continue;
                        }
                        match build_browse(paths, &mut recent) {
                            Ok(b) => {
                                browsing = Some(Browsing::new(b));
                                reviewing = None;
                                walking = None;
                                examining = None;
                                respond_json(request, &browse_payload(browsing.as_ref()));
                            }
                            Err(e) => {
                                eprintln!("warning: could not load the selected decks: {e}");
                                respond_status(request, 400);
                            }
                        }
                    }
                    None => respond_status(request, 400),
                }
            }
            // The focus drawer asks for a deck's stored topologies + region
            // heatmaps when it's selected. Read-only: open the deck's own store
            // transiently, never disturbing the active session store.
            (Method::Post, "/api/deck-topology") => {
                let dto = match read_selection(&mut request, &decks_dir, &recent) {
                    Some(sel) => {
                        match (
                            Deck::load(&sel.deck),
                            store_for(std::slice::from_ref(&sel.deck)),
                        ) {
                            (Ok(deck), Ok(s)) => {
                                let augment =
                                    AugmentCache::open(augment::augment_path_for(s.path()));
                                deck_topology_dto(&augment, &s, &deck, review_cfg)
                            }
                            _ => DeckTopologyDto::default(),
                        }
                    }
                    None => DeckTopologyDto::default(),
                };
                respond_json(request, &dto);
            }
            (Method::Post, "/api/deselect") => {
                reviewing = None;
                walking = None;
                browsing = None;
                // Back at the picker: read the global store again (loose-deck
                // badges live there, not in any workspace's store).
                if let Ok(s) = store_for(&[]) {
                    store = s;
                }
                respond_json(
                    request,
                    &review_state(reviewing.as_ref(), &store),
                );
            }
            (Method::Post, "/api/grade") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                match read_grade(&mut request) {
                    Some(grade) => {
                        r.session.grade(&mut store, grade, now_ms());
                        if let Err(e) = store.save() {
                            eprintln!("warning: could not save progress: {e}");
                        }
                        r.rotate_variant(); // a fresh phrasing for the next card
                        respond_json(
                            request,
                            &review_state(reviewing.as_ref(), &store),
                        );
                    }
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/skip") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.session.skip(&store, now_ms());
                r.rotate_variant(); // a fresh phrasing for the next card
                respond_json(
                    request,
                    &review_state(reviewing.as_ref(), &store),
                );
            }
            (Method::Post, "/api/acquire") => {
                // Acknowledge a never-seen card: record it at stage 1 (no grade)
                // and move on. Its first quiz comes back ~1 min later, this session.
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.session.acquire_current(&mut store, now_ms());
                if let Err(e) = store.save() {
                    eprintln!("warning: could not save progress: {e}");
                }
                r.rotate_variant(); // a fresh phrasing for the next card
                respond_json(
                    request,
                    &review_state(reviewing.as_ref(), &store),
                );
            }
            (Method::Post, "/api/check") => {
                let Some(r) = reviewing.as_ref() else {
                    respond_status(request, 409);
                    continue;
                };
                // Grade the typed lines against the current card — exact for
                // typing (tolerance 0), typo-tolerant for fuzzy. Like choose,
                // this only checks; the grade is applied on Continue.
                #[derive(Deserialize)]
                struct Body {
                    lines: Vec<String>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let result = body.and_then(|body| {
                    let card = r.session.current()?;
                    let reveal = card.reveal.unwrap_or_default();
                    let rung = store.get(card.id()).map(|s| s.rung).unwrap_or_default();
                    let mode = ladder::check_for(reveal, rung, card);
                    let tol = if mode == Mode::Typing { 0 } else { max_typos };
                    // Order-independent: a multi-item answer can be typed in any
                    // order, each line matched to its closest expected line.
                    let results: Vec<LineResultDto> =
                        grade_lines_unordered(&body.lines, &card.back, tol)
                            .into_iter()
                            .map(|r| LineResultDto {
                                input: r.input,
                                expected: r.expected,
                                passed: r.passed,
                                distance: r.distance,
                            })
                            .collect();
                    let passed = results.iter().all(|r| r.passed);
                    Some(CheckFeedbackDto { results, passed })
                });
                match result {
                    Some(f) => respond_json(request, &f),
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/choose") => {
                let Some(r) = reviewing.as_ref() else {
                    respond_status(request, 409);
                    continue;
                };
                // Just reports which option is correct (the question is rebuilt
                // from the card id, so it matches the one served). The grade is
                // applied later via /api/grade on Continue, so the session stays
                // on this card during the result — Remove still works on it.
                let picked = read_index(&mut request).and_then(|chosen| {
                    let card = r.session.current()?.clone();
                    let ai = r.augment.distractors(card.id());
                    let correct = choice::build(&card, r.session.cards(), card.id(), ai)?.correct;
                    Some((chosen, correct))
                });
                match picked {
                    Some((chosen, correct)) => respond_json(
                        request,
                        &ChooseFeedbackDto {
                            chosen,
                            correct,
                            passed: chosen == correct,
                        },
                    ),
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/remove") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let dropped = r.session.remove_current(&store, now_ms());
                if let Some(first) = dropped.first() {
                    let subject = first.subject.to_string();
                    let line = first.line;
                    for card in &dropped {
                        store.remove(card.id());
                    }
                    let _ = store.save();
                    r.files.remove_block(&subject, line);
                }
                respond_json(
                    request,
                    &review_state(reviewing.as_ref(), &store),
                );
            }
            // Promotes the current virtual (remediation) card into its deck
            // file (`store::promote_virtual` does the append-then-drop; the
            // schedule needs no transfer, since it already lives in
            // `store.cards` under the id the appended deck card hashes to, so
            // the promoted card keeps its earned schedule for free). A clean
            // 400 — never a panic — when the current card isn't virtual or
            // its deck file isn't known.
            (Method::Post, "/api/promote") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if !r.session.current_is_virtual(&store) {
                    respond_status(request, 400);
                    continue;
                }
                let Some(id) = r.session.current_id() else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(subject) = r.session.current().map(|c| c.subject.to_string()) else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(path) = r.files.paths.get(&subject).cloned() else {
                    respond_status(request, 400);
                    continue;
                };
                if store::promote_virtual(&mut store, id, &path).is_err() {
                    respond_status(request, 400);
                    continue;
                }
                r.session.poll(&store, now_ms());
                respond_json(
                    request,
                    &review_state(reviewing.as_ref(), &store),
                );
            }
            (Method::Post, "/api/restart") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.session.restart(&store, now_ms());
                r.rotate_variant(); // a fresh phrasing for the new session's first card
                respond_json(
                    request,
                    &review_state(reviewing.as_ref(), &store),
                );
            }
            // Ask Claude about the current card — runs the CLI on a background
            // thread (ask::spawn) and returns immediately; the page polls
            // `GET /api/ask` for the answer so the server loop never blocks.
            (Method::Post, "/api/ask") => {
                #[derive(Deserialize)]
                struct Body {
                    question: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let question = body.map(|b| b.question).filter(|q| !q.trim().is_empty());
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(q) = question {
                    r.start_ask(&ask_cfg, Some(q));
                }
                respond_json(request, &r.ask_dto(None, None));
            }
            // Condense the conversation into note lines appended to the deck.
            (Method::Post, "/api/ask/note") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.start_ask(&ask_cfg, None);
                respond_json(request, &r.ask_dto(None, None));
            }
            // Poll for a pending reply; the page calls this every ~400ms while
            // `thinking`.
            (Method::Get, "/api/ask") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let (status, error) = r.poll_ask();
                respond_json(request, &r.ask_dto(status, error));
            }
            // ── AI exam ───────────────────────────────────────────────────
            // Start an exam for one `% source:` deck: validate the name and
            // drill state, then spawn question generation on a background
            // thread; the page polls `GET /api/exam`.
            (Method::Post, "/api/exam/start") => {
                #[derive(Deserialize)]
                struct Body {
                    deck: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                // Include workspace members (by their qualified `<ws>/<file>`
                // name) so an exam can be started on a deck inside a workspace,
                // not just a top-level deck — mirroring `/api/select`.
                let mut known: HashMap<String, PathBuf> = HashMap::new();
                for e in picker::catalog(&decks_dir, &recent) {
                    for m in &e.members {
                        known.insert(m.name.clone(), m.path.clone());
                    }
                    known.insert(e.name, e.path);
                }
                let Some(path) = body.and_then(|b| known.get(&b.deck).cloned()) else {
                    respond_status(request, 400);
                    continue;
                };
                // The exam reads drill state and writes mastery/unlocks to the
                // deck's own store (a workspace's, or the global one).
                if let Ok(s) = store_for(std::slice::from_ref(&path)) {
                    store = s;
                }
                match Deck::load(&path) {
                    // Examable when it has an exam (a `% source:` fact deck, or a
                    // trace) and its `% requires:` are satisfied — drilled or not
                    // (you may test out early).
                    Ok(deck)
                        if deck.has_exam()
                            && !deck::is_locked(&deck, Some(decks_dir.as_path()), &store) =>
                    {
                        let strictness =
                            deck.settings.exam_strictness.unwrap_or(exam_cfg.strictness);
                        // A trace's exam is the graded compression (one fixed
                        // question), gated by the re-sit cooldown after a fail; a
                        // fact deck's exam generates questions from its source.
                        let sitting = if deck.is_trace() {
                            match trace::Trace::from_deck(&deck) {
                                Ok(t) => {
                                    if let Some(ms) = exam::cooldown_remaining_ms(
                                        &store,
                                        &deck.subject,
                                        exam_cfg.retry_cooldown_secs,
                                        now_ms(),
                                    ) {
                                        #[derive(Serialize)]
                                        struct Cooldown {
                                            cooldown_ms: u64,
                                        }
                                        respond_json(request, &Cooldown { cooldown_ms: ms });
                                        continue;
                                    }
                                    exam::Sitting::start_trace(
                                        t.description.clone(),
                                        t.compression_rubric(),
                                        deck.subject.clone(),
                                        strictness,
                                        exam_cfg.clone(),
                                        ask_cfg.clone(),
                                    )
                                }
                                Err(_) => {
                                    respond_status(request, 409);
                                    continue;
                                }
                            }
                        } else {
                            exam::Sitting::start(
                                &deck,
                                strictness,
                                exam_cfg.clone(),
                                ask_cfg.clone(),
                            )
                        };
                        let ex = Examining {
                            sitting,
                            deck_path: path,
                        };
                        let dto = exam_dto(&ex, &decks_dir);
                        examining = Some(ex);
                        respond_json(request, &dto);
                    }
                    _ => respond_status(request, 409),
                }
            }
            // Poll the exam: advance any finished background call, return state.
            (Method::Get, "/api/exam") => {
                let Some(ex) = examining.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                // A workspace member's exam remediation honors its own
                // `alix.local.toml` retirement cap, same as its review session.
                let retire_after_days = review_cfg.for_deck(&ex.deck_path).retire_after_days;
                ex.sitting.poll(&mut store, now_ms(), retire_after_days);
                respond_json(request, &exam_dto(ex, &decks_dir));
            }
            // Save the current answer and (optionally) navigate to another question.
            (Method::Post, "/api/exam/answer") => {
                #[derive(Deserialize)]
                struct Body {
                    text: String,
                    goto: Option<usize>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(ex) = examining.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(b) = body {
                    ex.sitting.set_answer(b.text);
                    if let Some(i) = b.goto {
                        ex.sitting.goto(i);
                    }
                }
                respond_json(request, &exam_dto(ex, &decks_dir));
            }
            // Save the last answer and submit everything for grading.
            (Method::Post, "/api/exam/grade") => {
                #[derive(Deserialize)]
                struct Body {
                    text: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(ex) = examining.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(b) = body {
                    ex.sitting.set_answer(b.text);
                }
                ex.sitting.submit();
                respond_json(request, &exam_dto(ex, &decks_dir));
            }
            // On a fail, generate remediation cards into the store as virtual cards.
            (Method::Post, "/api/exam/remediate") => {
                let Some(ex) = examining.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                ex.sitting.remediate();
                respond_json(request, &exam_dto(ex, &decks_dir));
            }
            // Leave the exam, back to the deck list / summary.
            (Method::Post, "/api/exam/close") => {
                examining = None;
                if let Ok(s) = store_for(&[]) {
                    store = s;
                }
                respond_json(
                    request,
                    &review_state(reviewing.as_ref(), &store),
                );
            }
            // ── Deck augmentation (the picker's "Augment" action, decks only) ──
            // Open a deck's Augment screen and report what its cache holds. Resolves
            // the deck through the catalog (incl. workspace members) like the exam.
            (Method::Post, "/api/augment/open") => {
                #[derive(Deserialize)]
                struct Body {
                    deck: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let mut known: HashMap<String, PathBuf> = HashMap::new();
                for e in picker::catalog(&decks_dir, &recent) {
                    for m in &e.members {
                        known.insert(m.name.clone(), m.path.clone());
                    }
                    known.insert(e.name, e.path);
                }
                let Some((name, path)) =
                    body.and_then(|b| known.get(&b.deck).cloned().map(|p| (b.deck, p)))
                else {
                    respond_status(request, 400);
                    continue;
                };
                // The cache lives beside the deck's own store (a workspace's, or the
                // global one), mirroring how review reads it.
                if let Ok(s) = store_for(std::slice::from_ref(&path)) {
                    store = s;
                }
                match Deck::load(&path) {
                    Ok(deck) => {
                        let aug = Augmenting::open(
                            name,
                            deck.cards,
                            augment::augment_path_for(store.path()),
                        );
                        let dto = aug.dto();
                        augmenting = Some(aug);
                        respond_json(request, &dto);
                    }
                    Err(_) => respond_status(request, 409),
                }
            }
            // Start fill-the-gaps generation for one target (a costed background
            // call); the page polls `GET /api/augment`.
            (Method::Post, "/api/augment/generate") => {
                #[derive(Deserialize)]
                struct Body {
                    target: String,
                    with: Option<String>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(aug) = augmenting.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(b) = body {
                    let guidance = b
                        .with
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    aug.generate(&b.target, guidance, &ai_cfg, &ask_cfg);
                }
                respond_json(request, &aug.dto());
            }
            // Poll the in-flight generation: apply a finished outcome, return state.
            (Method::Get, "/api/augment") => {
                let Some(aug) = augmenting.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                aug.poll();
                respond_json(request, &aug.dto());
            }
            // Remove a target's augmentations (or `all`) for this deck.
            (Method::Post, "/api/augment/remove") => {
                #[derive(Deserialize)]
                struct Body {
                    target: String,
                    topology: Option<String>,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(aug) = augmenting.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(b) = body {
                    aug.remove(&b.target, b.topology.as_deref());
                }
                respond_json(request, &aug.dto());
            }
            // Leave the Augment screen, back to the picker (reset to the global store).
            (Method::Post, "/api/augment/close") => {
                augmenting = None;
                if let Ok(s) = store_for(&[]) {
                    store = s;
                }
                respond_json(
                    request,
                    &review_state(reviewing.as_ref(), &store),
                );
            }
            // ── Trace walk (a single trace picked from the selection screen) ──
            // The web trace-walk flow (predict → reveal → grade), guarded on `walking`.
            (Method::Get, "/api/walk") => {
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                w.poll();
                respond_json(request, &walk_dto(w));
            }
            (Method::Post, "/api/walk/predict") => {
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                #[derive(Deserialize)]
                struct Body {
                    text: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                if let Some(b) = body {
                    w.walk.predict(b.text);
                    w.start_grade();
                }
                respond_json(request, &walk_dto(w));
            }
            (Method::Post, "/api/walk/grade") => {
                let self_delta = read_delta(&mut request);
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let delta = w.grade_result.as_ref().map(|(d, _)| *d).or(self_delta);
                match delta {
                    Some(delta) => {
                        w.walk.grade(&mut store, delta, now_ms());
                        if let Err(e) = store.save() {
                            eprintln!("warning: could not save progress: {e}");
                        }
                        w.clear_grade();
                        respond_json(request, &walk_dto(w));
                    }
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/walk/restart") => {
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let fresh = Walk::new(w.walk.trace().clone());
                let grade = w.grade.take();
                *w = Walking::new(fresh, grade);
                respond_json(request, &walk_dto(w));
            }
            // Ask-Claude about the current checkpoint — the same tutor a review
            // uses (its subject is the checkpoint). Runs on a background thread;
            // the page polls `GET /api/walk/ask`.
            (Method::Post, "/api/walk/ask") => {
                #[derive(Deserialize)]
                struct Body {
                    question: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let question = body.map(|b| b.question).filter(|q| !q.trim().is_empty());
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(q) = question {
                    w.start_ask(&ask_cfg, Some(q));
                }
                respond_json(request, &w.ask_dto(None, None));
            }
            // Condense the conversation into a `!` note on the checkpoint.
            (Method::Post, "/api/walk/ask/note") => {
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                w.start_ask(&ask_cfg, None);
                respond_json(request, &w.ask_dto(None, None));
            }
            (Method::Get, "/api/walk/ask") => {
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let (status, error) = w.poll_ask();
                respond_json(request, &w.ask_dto(status, error));
            }
            // Back to decks: abandon the walk and return to the picker (global store).
            (Method::Post, "/api/walk/leave") => {
                walking = None;
                if let Ok(s) = store_for(&[]) {
                    store = s;
                }
                respond_status(request, 204);
            }
            _ => respond_status(request, 404),
        }
    }
    Ok(())
}

// ── Trace walks (`alix trace --serve`) ──────────────────────────────────────
//
// A single walk of one trace deck, mirroring the terminal `run_walk`: predict →
// reveal a live excerpt → grade → compress. There is no deck-selection screen
// (one deck, one walk). The frontend-agnostic `Walk` state machine carries the
// logic; this is a thin web reader over it, exactly like the TUI consumer. Live
// Claude grading (`--grade`) is the only async step, so it runs on a background
// thread and the page polls `GET /api/walk` while `thinking`, like the exam.

/// The server's live trace-walk state. Holds the [`Walk`], the (optional) live
/// grading config, and the in-flight/just-finished Claude grade for the current
/// reveal.
struct Walking {
    walk: Walk,
    /// `Some` in `--grade` mode: the `[ask]` config a background grade uses
    /// (grading runs at the tutor tier, not trace's heavy build defaults).
    grade: Option<AskConfig>,
    /// A background Claude grade in flight for the current reveal.
    pending: Option<Receiver<Result<(Delta, String), String>>>,
    /// The resolved Claude grade for the current reveal (verdict + feedback).
    grade_result: Option<(Delta, String)>,
    /// A failed Claude grade — the reveal falls back to self-grading.
    grade_error: Option<String>,
    /// Ask-Claude tutor for the current checkpoint — the same machinery a review
    /// uses, its subject the checkpoint instead of a card.
    ask: Ask,
}

impl Walking {
    fn new(walk: Walk, grade: Option<AskConfig>) -> Self {
        Walking {
            walk,
            grade,
            pending: None,
            grade_result: None,
            grade_error: None,
            ask: Ask::new(),
        }
    }

    /// The current checkpoint as a tutor [`Card`]: front = the predict prompt,
    /// back = the key points, note = the live source excerpt + the connecting
    /// insight. Its `id()` matches the checkpoint's `card_id` (both hash subject +
    /// back), so the transcript aligns per checkpoint.
    fn checkpoint_card(&self) -> Option<Card> {
        let trace = self.walk.trace();
        let cp = self.walk.checkpoint()?;
        let mut note = String::new();
        if let Ok(ex) = trace.excerpt(cp) {
            note.push_str("Source excerpt:\n");
            for (n, line) in &ex.lines {
                note.push_str(&format!("{n}: {line}\n"));
            }
        }
        if let Some(insight) = &cp.note {
            if !note.is_empty() {
                note.push('\n');
            }
            note.push_str(insight);
        }
        Some(Card::plain(
            Arc::from(trace.subject.as_str()),
            cp.prompt.clone(),
            cp.points.clone(),
            (!note.is_empty()).then_some(note),
            cp.line,
        ))
    }

    /// Starts an ask-Claude call about the current checkpoint (or condenses into a
    /// note with `question: None`). No-op off a checkpoint (the done screen).
    fn start_ask(&mut self, cfg: &AskConfig, question: Option<String>) -> bool {
        let Some(card) = self.checkpoint_card() else {
            return false;
        };
        // Ground the walk tutor in the trace's live source (opt-in), with the
        // current checkpoint's frozen excerpt as the anchor.
        let root = cfg
            .source_access
            .then(|| self.walk.trace().origin.clone())
            .flatten();
        let frozen = root.as_ref().and_then(|_| {
            let c = self.walk.checkpoint()?;
            self.walk.trace().frozen_block(c)
        });
        let live_root = root.as_deref().filter(|r| r.exists());
        self.ask
            .start(cfg, &card, &[], live_root, frozen.as_deref(), question)
    }

    /// Drains a finished ask reply; a "save note" condense appends a `!` line to
    /// the current checkpoint in the trace deck file.
    fn poll_ask(&mut self) -> (Option<String>, Option<String>) {
        self.ask.align(self.walk.checkpoint().map(|c| c.card_id));
        let deck_path = self.walk.trace().deck_path.clone();
        self.ask.poll(|card, notes| {
            crate::deck::append_note(&deck_path, card.line, notes).map_err(|e| e.to_string())
        })
    }

    fn ask_dto(&self, status: Option<String>, error: Option<String>) -> AskDto {
        self.ask.dto(status, error)
    }

    /// After a prediction, kick off a background Claude grade — a no-op outside
    /// `--grade` mode. Clears any prior grade state for the fresh reveal.
    fn start_grade(&mut self) {
        self.clear_grade();
        let Some(ask_cfg) = self.grade.as_ref() else {
            return;
        };
        let Some(checkpoint) = self.walk.checkpoint() else {
            return;
        };
        let prediction = self
            .walk
            .prediction(self.walk.current_index())
            .unwrap_or("")
            .to_string();
        let rx = trace::spawn_grade(checkpoint.clone(), prediction, ask_cfg.clone());
        self.pending = Some(rx);
    }

    /// Drains a finished background grade into `grade_result`/`grade_error`.
    fn poll(&mut self) {
        let Some(rx) = &self.pending else { return };
        match rx.try_recv() {
            Ok(Ok((delta, feedback))) => {
                self.grade_result = Some((delta, feedback));
                self.pending = None;
            }
            Ok(Err(e)) => {
                self.grade_error = Some(e);
                self.pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.grade_error = Some("the grading thread ended unexpectedly".to_string());
                self.pending = None;
            }
        }
    }

    /// Clears all grade state when leaving a reveal.
    fn clear_grade(&mut self) {
        self.pending = None;
        self.grade_result = None;
        self.grade_error = None;
    }
}

/// One checkpoint as a node on the path rail.
#[derive(Serialize)]
struct HopDto {
    prompt: String,
    /// `passed` | `partly` | `failed` once judged; `null` while unwalked.
    delta: Option<&'static str>,
    /// The hop currently being predicted or revealed.
    current: bool,
}

/// A revealed source excerpt for the browser — line-numbered, contiguous.
#[derive(Debug, Serialize)]
struct ExcerptDto {
    path: String,
    lines: Vec<LineDto>,
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct LineDto {
    n: usize,
    text: String,
}

/// The walk tally shown on the done screen.
#[derive(Serialize)]
struct SummaryDto {
    passed: usize,
    partly: usize,
    failed: usize,
    /// 1-based hop numbers judged partly or failed.
    weak: Vec<usize>,
    total: usize,
}

/// The trace-walk payload sent to the browser. The page renders sub-views off
/// `phase` and polls `GET /api/walk` while `thinking` (a live grade in flight).
#[derive(Serialize)]
struct WalkDto {
    /// Discriminates this trace-walk payload from the review [`StateDto`] for the
    /// single client dispatcher (`isWalk`): always `"walk"`.
    kind: &'static str,
    phase: &'static str,
    description: String,
    source: Option<String>,
    total: usize,
    /// 1-based index of the hop being walked.
    current: usize,
    /// The path rail — one node per checkpoint.
    path: Vec<HopDto>,
    // predict + reveal
    prompt: Option<String>,
    givens: Vec<String>,
    locator: Option<String>,
    /// What the learner predicted (shown on reveal).
    prediction: Option<String>,
    // reveal
    excerpt: Option<ExcerptDto>,
    excerpt_error: Option<String>,
    points: Vec<String>,
    note: Option<String>,
    /// `--grade` mode: Claude judges instead of the learner.
    auto_grade: bool,
    /// A live grade is in flight.
    thinking: bool,
    verdict: Option<&'static str>,
    feedback: Option<String>,
    /// A live grade failed — the reveal offers self-grading instead.
    grade_error: Option<String>,
    // done
    summary: Option<SummaryDto>,
}

fn walk_phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Predict => "predict",
        Phase::Reveal => "reveal",
        Phase::Done => "done",
    }
}

fn delta_name(delta: Delta) -> &'static str {
    match delta {
        Delta::Passed => "passed",
        Delta::Partial => "partly",
        Delta::Failed => "failed",
    }
}

fn excerpt_dto(excerpt: &Excerpt) -> ExcerptDto {
    ExcerptDto {
        path: excerpt.path.display().to_string(),
        lines: excerpt
            .lines
            .iter()
            .map(|(n, text)| LineDto {
                n: *n,
                text: text.clone(),
            })
            .collect(),
        truncated: excerpt.truncated,
    }
}

/// Serializes the current walk state for the browser.
fn walk_dto(w: &Walking) -> WalkDto {
    let walk = &w.walk;
    let trace = walk.trace();
    let phase = walk.phase();
    let on_a_hop = matches!(phase, Phase::Predict | Phase::Reveal);

    let path = trace
        .checkpoints
        .iter()
        .enumerate()
        .map(|(i, c)| HopDto {
            prompt: c.prompt.clone(),
            delta: walk.delta(i).map(delta_name),
            current: on_a_hop && i == walk.current_index(),
        })
        .collect();

    let mut dto = WalkDto {
        kind: "walk",
        phase: walk_phase_name(phase),
        description: trace.description.clone(),
        source: trace.source.clone(),
        total: walk.total(),
        current: walk.current_index() + 1,
        path,
        prompt: None,
        givens: Vec::new(),
        locator: None,
        prediction: None,
        excerpt: None,
        excerpt_error: None,
        points: Vec::new(),
        note: None,
        auto_grade: w.grade.is_some(),
        thinking: w.pending.is_some(),
        verdict: w.grade_result.as_ref().map(|(d, _)| d.label()),
        feedback: w.grade_result.as_ref().map(|(_, f)| f.clone()),
        grade_error: w.grade_error.clone(),
        summary: None,
    };

    match phase {
        Phase::Predict => {
            if let Some(c) = walk.checkpoint() {
                dto.prompt = Some(c.prompt.clone());
                dto.givens = c.givens.clone();
                dto.locator = c.locator.clone();
            }
        }
        Phase::Reveal => {
            if let Some(c) = walk.checkpoint() {
                dto.prompt = Some(c.prompt.clone());
                dto.givens = c.givens.clone();
                dto.locator = c.locator.clone();
                dto.points = c.points.clone();
                dto.note = c.note.clone();
                match trace.excerpt(c) {
                    Ok(ex) => {
                        // For a frozen-snapshot asset, relabel the excerpt + the
                        // "at" label to the ORIGINAL source (`caching.rs:106-120`),
                        // so the gutter shows real line numbers, not the asset's.
                        let (ex, label) = trace::relabel_for_display(ex, c.at_origin.as_deref());
                        if let Some(label) = label {
                            dto.locator = Some(label);
                        }
                        dto.excerpt = Some(excerpt_dto(&ex));
                    }
                    Err(e) => dto.excerpt_error = Some(format!("{e:#}")),
                }
            }
            dto.prediction = walk
                .prediction(walk.current_index())
                .map(str::to_string)
                .filter(|p| !p.is_empty());
        }
        Phase::Done => {
            let s = walk.summary();
            dto.summary = Some(SummaryDto {
                passed: s.passed,
                partly: s.partly,
                failed: s.failed,
                weak: s.weak.iter().map(|i| i + 1).collect(),
                total: walk.total(),
            });
        }
    }
    dto
}

/// Parses a `{"delta": "g"|"p"|"m"}` self-grade POST body.
fn read_delta(request: &mut Request) -> Option<Delta> {
    #[derive(Deserialize)]
    struct Body {
        delta: String,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    Delta::from_key(body.delta.chars().next()?)
}

/// The server's live browse state once decks are chosen. Its absence (`None`)
/// means the deck-selection phase.
struct Browsing {
    cards: Vec<Card>,
    label: String,
    images: HashMap<String, PathBuf>,
}

impl Browsing {
    fn new(build: CardsBuild) -> Self {
        let images = collect_images(&build.cards);
        Self {
            cards: build.cards,
            label: build.label,
            images,
        }
    }
}

/// Serializes the current browse phase for the page: the cards in browse phase,
/// or an empty list flagged `phase: "select"` for the deck-selection screen.
fn browse_payload(browsing: Option<&Browsing>) -> BrowseDto {
    match browsing {
        Some(b) => BrowseDto {
            phase: "browse",
            label: b.label.clone(),
            cards: b.cards.iter().map(card_dto).collect(),
        },
        None => BrowseDto {
            phase: "select",
            label: "select decks".to_string(),
            cards: Vec::new(),
        },
    }
}

/// The path part of a request URL, without any `?query`.
fn request_path(request: &Request) -> String {
    request.url().split('?').next().unwrap_or("").to_string()
}

/// Whether a request may proceed. Only `/api/*` is guarded; the HTML shell,
/// theme assets, and images stay open so the browser can bootstrap its token
/// from the `?token=` URL. No token configured (the localhost default) → open.
fn is_authorized(
    path: &str,
    auth_header: Option<&str>,
    query_token: Option<&str>,
    token: Option<&str>,
) -> bool {
    let Some(token) = token else { return true };
    if !path.starts_with("/api/") {
        return true;
    }
    let presented = auth_header
        .and_then(|h| h.strip_prefix("Bearer "))
        .or(query_token);
    presented.is_some_and(|p| ct_eq(p.as_bytes(), token.as_bytes()))
}

/// Constant-time byte comparison, so checking the pairing token doesn't leak it
/// through timing. Length is not secret — a length mismatch returns early.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A request header's value by name (case-insensitive), if present.
fn header_value<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}

/// A query parameter's value from a full request URL (`/path?k=v&…`).
fn query_param(url: &str, key: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

/// Builds the state payload. In the select phase (`reviewing` is `None`) it
/// reports `phase: "select"` with no card; otherwise it serializes the live
/// session and store. For choice mode it also builds the options, seeded by the
/// card id so they are stable across the `/api/state` and `/api/choose`
/// requests without any server-side caching.
fn review_state(reviewing: Option<&Reviewing>, store: &Store) -> StateDto {
    let Some(r) = reviewing else {
        return StateDto {
            kind: "review",
            phase: "select",
            card: None,
            choices: None,
            keypoints: None,
            acquire: false,
            mode: mode_name(Mode::default()),
            input: input_name(Input::default()),
            remaining: 0,
            initial: 0,
            reviews: 0,
            passed: 0,
            failed: 0,
            histogram: [0; 6],
            exam_due: Vec::new(),
            can_restart: false,
            promotable: false,
            top_stage: crate::store::MAX_STAGE,
            label: "select decks".to_string(),
        };
    };
    let session = &r.session;
    let card = session.current();
    // The frontier depth rung the card is currently scheduled at (persisted in
    // the store; an unreviewed card defaults to `Rung::default()`). Feeds the
    // concrete check (`mode`) below via `check_for`.
    let rung = card
        .map(|c| store.get(c.id()).map(|s| s.rung).unwrap_or_default())
        .unwrap_or_default();
    // The concrete check derives from the card's authored `% reveal:` method and
    // its current stored rung (spec §8); depth is the deck target, not authored.
    let mode = card
        .map(|c| {
            let reveal = c.reveal.unwrap_or_default();
            ladder::check_for(reveal, rung, c)
        })
        .unwrap_or_default();
    // A never-seen card is *acquired* (an attempt, then reveal), not quizzed cold.
    let acquire = session.current_unseen(store);
    let choices = if acquire {
        // First encounter: a recognition question only under the strict bar (atomic
        // answer + a full set of cached AI distractors); otherwise recall-then-reveal
        // (`choices: None`). Same `c.id()` seed `/api/choose` rebuilds from.
        card.and_then(|c| {
            choice::recognition_question(c, session.cards(), c.id(), r.augment.distractors(c.id()))
                .map(|q| q.options)
        })
    } else if mode == Mode::Choice {
        card.and_then(|c| {
            choice::build(c, session.cards(), c.id(), r.augment.distractors(c.id()))
                .map(|q| q.options)
        })
    } else {
        None
    };
    // Explain mode with cached key points reveals them as a checklist whose
    // coverage derives the grade; any other mode keeps the plain reveal. Not on a
    // first encounter — acquiring just reveals the answer.
    let keypoints = if !acquire && mode == Mode::Explain {
        card.and_then(|c| r.augment.keypoints(c.id()).map(<[String]>::to_vec))
    } else {
        None
    };
    // On a finished session, surface any deck that just reached "exam due" so the
    // summary can offer to sit it. Only computed when finished (it reloads decks).
    let finished = session.is_finished();
    let exam_due = if finished {
        let mut due: Vec<String> = r
            .files
            .paths
            .iter()
            .filter_map(|(subject, path)| {
                Deck::load(path)
                    .ok()
                    .filter(|d| d.state(store) == DeckState::ExamDue)
                    .map(|_| subject.clone())
            })
            .collect();
        due.sort();
        due
    } else {
        Vec::new()
    };
    // Resolve a fact card's `% at:` citation against its deck's source base, so
    // the browser can show it on reveal (the same live read a trace walk does).
    let card_with_citation = card.map(|c| {
        let mut dto = card_dto(c);
        // The "where am I" region breadcrumb, when topology-ordered. A cache can
        // hold several like-named topologies (decks sharing a store), so pick the
        // one of this principle that actually contains the current card — the card
        // itself disambiguates which deck's topology applies.
        if let Some(name) = &r.topology_name
            && let Some((topo, regions, current)) = r
                .augment
                .topologies()
                .iter()
                .filter(|t| t.name == *name)
                .find_map(|t| t.region_path(c.id()).map(|(rg, cur)| (t, rg, cur)))
        {
            dto.crumb = Some(CrumbDto {
                regions: regions.into_iter().map(str::to_string).collect(),
                current,
                cells: topo
                    .regions
                    .iter()
                    .map(|reg| crate::session::card_strengths(&reg.cards, store, now_ms()))
                    .collect(),
            });
        }
        if let Some(locator) = c.at.as_deref() {
            dto.at = Some(locator.to_string());
            if let Some(base) = r.source_bases.get(&*c.subject) {
                match base.excerpt(locator) {
                    Ok(ex) => {
                        // Relabel a frozen-snapshot asset to its real source +
                        // line numbers (the `% at:` ` from <file>:<lines>` origin),
                        // so the citation reads `store.rs:36-66`, not `01.rs`, 1-N.
                        let (ex, label) = trace::relabel_for_display(ex, c.at_origin.as_deref());
                        if let Some(label) = label {
                            dto.at = Some(label);
                        }
                        dto.citation = Some(excerpt_dto(&ex));
                    }
                    Err(e) => dto.citation_error = Some(format!("{e:#}")),
                }
            }
        }
        dto
    });
    StateDto {
        kind: "review",
        // The session-end signal is a phase value (matching the walk's `done`),
        // not a side flag.
        phase: if finished { "done" } else { "review" },
        card: card_with_citation,
        choices,
        keypoints,
        acquire,
        mode: mode_name(mode),
        input: input_name(card.and_then(|c| c.input).unwrap_or_default()),
        remaining: session.remaining(),
        initial: session.initial_size,
        reviews: session.stats.reviews,
        passed: session.stats.passed,
        failed: session.stats.failed,
        histogram: session.stage_histogram(store),
        exam_due,
        can_restart: session.has_due_now(store, now_ms()),
        promotable: session.current_is_virtual(store),
        top_stage: session.top_stage(),
        label: r.label.clone(),
    }
}

/// A `DeckState` as the machine-readable string the page styles rows by.
fn state_name(s: DeckState) -> &'static str {
    match s {
        DeckState::NotStarted => "new",
        DeckState::Started => "started",
        DeckState::Finished => "finished",
        DeckState::ExamDue => "examdue",
    }
}

/// A single loose deck as a selection row, its badge/lock/gating from the shared
/// [`picker::deck_status`] so it reads exactly like the TUI picker.
fn deck_item_dto(
    e: &picker::DeckEntry,
    store: &Store,
    decks_dir: &Path,
    with_lock: bool,
    augment: &AugmentCache,
    review: ReviewConfig,
) -> DeckItemDto {
    let recent = e.last_used_ms.is_some();
    match Deck::load(&e.path) {
        Ok(deck) => {
            let s = picker::deck_status(&deck, store, Some(decks_dir), with_lock, review);
            let deck_ids: HashSet<u64> = deck.cards.iter().map(|c| c.id()).collect();
            DeckItemDto {
                name: e.name.clone(),
                label: e.label.clone(),
                meta: Some(s.badge),
                state: state_name(s.state),
                locked: s.locked,
                reviewable: s.reviewable,
                mastered: s.mastered,
                is_trace: s.is_trace,
                examable: s.examable,
                has_exam: s.has_exam,
                recent,
                is_workspace: false,
                description: None,
                members: Vec::new(),
                path: e.path_hint.clone(),
                icon: None,
                icon_svg: false,
                has_topology: augment.has_topology_for(&deck_ids),
            }
        }
        // A deck that fails to load stays launchable so the error surfaces.
        Err(_) => DeckItemDto {
            name: e.name.clone(),
            label: e.label.clone(),
            meta: None,
            state: "new",
            locked: false,
            reviewable: true,
            mastered: false,
            is_trace: false,
            examable: false,
            has_exam: false,
            recent,
            is_workspace: false,
            description: None,
            members: Vec::new(),
            path: e.path_hint.clone(),
            icon: None,
            icon_svg: false,
            has_topology: false,
        },
    }
}

/// A workspace/folder's members as an unlock dependency tree (the drill-in
/// list): each member nests under the `% requires:` that gates it, siblings
/// startable-first, carrying a `depth` for indentation. Badges/locks come from
/// the workspace's own store (a real workspace) or the global store (a plain
/// folder), matching what a session will write.
fn workspace_members(
    e: &picker::DeckEntry,
    decks_dir: &Path,
    with_lock: bool,
    review: ReviewConfig,
) -> Vec<MemberDto> {
    // Member badges reflect this workspace's personal pacing override, if any.
    let review = review.for_workspace(&e.path);
    let store = if crate::workspace::is_workspace(&e.path) {
        Store::open(crate::workspace::store_path(&e.path)).ok()
    } else {
        crate::store::default_store_path().and_then(|p| Store::open(p).ok())
    };
    let paths: Vec<PathBuf> = e.members.iter().map(|m| m.path.clone()).collect();
    // The workspace's own sidecar tells each member whether it has a focus
    // drawer (topology); opened once, alongside the status pass.
    let augment = store
        .as_ref()
        .map(|s| AugmentCache::open(augment::augment_path_for(s.path())));
    // Load each member deck once, deriving both its status and whether it has a
    // topology from the same parse.
    let loaded: Vec<(Option<picker::DeckStatus>, bool)> = paths
        .iter()
        .map(|p| {
            let deck = Deck::load(p).ok();
            let status = match (store.as_ref(), deck.as_ref()) {
                (Some(st), Some(d)) => Some(picker::deck_status(
                    d,
                    st,
                    Some(decks_dir),
                    with_lock,
                    review,
                )),
                _ => None,
            };
            let has_topology = match (augment.as_ref(), deck.as_ref()) {
                (Some(a), Some(d)) => {
                    let ids: HashSet<u64> = d.cards.iter().map(|c| c.id()).collect();
                    a.has_topology_for(&ids)
                }
                _ => false,
            };
            (status, has_topology)
        })
        .collect();
    // Order siblings startable-first (blocked = locked, or — when gating —
    // nothing to review), then by label, like the TUI drill-in.
    let parent = picker::member_parents(&paths, decks_dir);
    let key: Vec<(bool, String)> = e
        .members
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let blocked = loaded[i]
                .0
                .as_ref()
                .is_some_and(|s| s.locked || (with_lock && !s.reviewable));
            (blocked, m.label.clone())
        })
        .collect();
    picker::dependency_forest(&parent, &key)
        .into_iter()
        .map(|(i, prefix)| {
            let m = &e.members[i];
            // Each tree branch segment is three columns wide (see picker).
            let depth = prefix.chars().count() / 3;
            let has_topology = loaded[i].1;
            match &loaded[i].0 {
                Some(s) => MemberDto {
                    name: m.name.clone(),
                    label: m.label.clone(),
                    meta: Some(s.badge.clone()),
                    state: state_name(s.state),
                    locked: s.locked,
                    reviewable: s.reviewable,
                    mastered: s.mastered,
                    is_trace: s.is_trace,
                    examable: s.examable,
                    has_exam: s.has_exam,
                    depth,
                    tree: prefix.clone(),
                    has_topology,
                },
                None => MemberDto {
                    name: m.name.clone(),
                    label: m.label.clone(),
                    meta: None,
                    state: "new",
                    locked: false,
                    reviewable: true,
                    mastered: false,
                    is_trace: false,
                    examable: false,
                    has_exam: false,
                    depth,
                    tree: prefix.clone(),
                    has_topology,
                },
            }
        })
        .collect()
}

/// Builds the deck-selection catalog in the TUI's three sections — workspaces
/// (each with its last-progress time), recent loose decks, and plain folders —
/// each deck's badge/lock from `store`. `with_lock` is false for the browse
/// screen: locking gates *review* only, so any deck is browsable.
/// The picker icon URL for a resolved icon path, registering it in the launcher
/// image map so `/img/<key>` can serve it. Returns the URL and whether it is an
/// SVG (a mask) or a raster (`<img>`).
fn icon_field(icon: Option<&Path>, icons: &mut HashMap<String, PathBuf>) -> (Option<String>, bool) {
    match icon {
        Some(path) => {
            let key = img_key(path);
            icons.insert(key.clone(), path.to_path_buf());
            let is_svg = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
            (Some(format!("/img/{key}")), is_svg)
        }
        None => (None, false),
    }
}

fn deck_catalog(
    decks_dir: &Path,
    recent: &RecentDecks,
    store: &Store,
    with_lock: bool,
    icons: &mut HashMap<String, PathBuf>,
    review: ReviewConfig,
) -> DeckListDto {
    let mut workspaces = Vec::new();
    let mut recent_decks = Vec::new();
    let mut folders = Vec::new();
    // Opened once for the whole catalog: the global store's sidecar tells each
    // loose deck whether it has a focus drawer (topology).
    let augment = AugmentCache::open(augment::augment_path_for(store.path()));
    for e in picker::catalog(decks_dir, recent) {
        // A workspace/folder row: its members open on click; it has no state of
        // its own. A folder with an `alix.toml` is a workspace (shown with its
        // last-progress time); without one it's a plain folder.
        if e.is_workspace {
            let is_ws = crate::workspace::is_workspace(&e.path);
            let members = workspace_members(&e, decks_dir, with_lock, review);
            let meta = if is_ws {
                match picker::workspace_last_progress(&e.path) {
                    Some(when) => format!("{} decks · {when}", members.len()),
                    None => format!("{} decks", members.len()),
                }
            } else {
                format!("{} decks", members.len())
            };
            let (icon, icon_svg) = icon_field(e.icon.as_deref(), icons);
            let dto = DeckItemDto {
                meta: Some(meta),
                state: if is_ws { "workspace" } else { "folder" },
                locked: false,
                reviewable: true,
                mastered: false,
                is_trace: false,
                examable: false,
                has_exam: false,
                recent: e.last_used_ms.is_some(),
                is_workspace: true,
                description: e.description,
                members,
                path: e.path_hint,
                name: e.name,
                label: e.label,
                icon,
                icon_svg,
                has_topology: false,
            };
            if is_ws {
                workspaces.push(dto);
            } else {
                folders.push(dto);
            }
            continue;
        }
        // A loose deck inside a workspace belongs to it — reached by opening the
        // workspace, so it isn't listed loose in Recent.
        if e.path.parent().is_some_and(crate::workspace::is_workspace) {
            continue;
        }
        recent_decks.push(deck_item_dto(
            &e, store, decks_dir, with_lock, &augment, review,
        ));
    }
    DeckListDto {
        workspaces,
        recent: recent_decks,
        folders,
    }
}

/// Parses a `{"decks":[name,…]}` selection and resolves each name to its deck
/// path via the live catalog. Returns `None` (→ 400) for an empty or malformed
/// body, or any name not in the catalog — so no filesystem path is ever built
/// from request input, keeping selection safe under `--lan`.
/// A deck chosen from the picker, optionally scoped by the focus drawer to one
/// topology and/or region.
struct Selection {
    deck: PathBuf,
    topology: Option<String>,
    region: Option<String>,
}

fn read_selection(
    request: &mut Request,
    decks_dir: &Path,
    recent: &RecentDecks,
) -> Option<Selection> {
    #[derive(Deserialize)]
    struct Body {
        deck: String,
        #[serde(default)]
        topology: Option<String>,
        #[serde(default)]
        region: Option<String>,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    if body.deck.is_empty() {
        return None;
    }
    // The resolution map includes top-level decks/workspaces and every
    // workspace's members (by their qualified `<workspace>/<file>` key), so a
    // member selection from inside a workspace resolves safely too. An unknown or
    // crafted name maps to nothing, so it's rejected rather than hitting the disk.
    let mut known: HashMap<String, PathBuf> = HashMap::new();
    for e in picker::catalog(decks_dir, recent) {
        for m in &e.members {
            known.insert(m.name.clone(), m.path.clone());
        }
        known.insert(e.name, e.path);
    }
    Some(Selection {
        deck: resolve_name(&body.deck, &known)?,
        topology: body.topology,
        region: body.region,
    })
}

/// Maps a requested deck name to its catalog path, or `None` if it isn't known —
/// so an unknown or crafted name (e.g. a traversal attempt) is rejected rather
/// than reaching the filesystem.
fn resolve_name(name: &str, known: &HashMap<String, PathBuf>) -> Option<PathBuf> {
    known.get(name).cloned()
}

/// Builds the focus-drawer payload for a `deck`: its own stored topologies (the
/// cache can be shared by several decks on one store, so they're scoped by card
/// membership), each region's per-card strength heatmap and due/new count, and
/// the whole-deck due count — all read against the deck's `store`.
fn deck_topology_dto(
    augment: &AugmentCache,
    store: &Store,
    deck: &Deck,
    review: ReviewConfig,
) -> DeckTopologyDto {
    // A workspace member's due counts honor its `alix.local.toml` pacing override.
    let review = review.for_deck(&deck.path);
    let by_id: HashMap<u64, &Card> = deck.cards.iter().map(|c| (c.id(), c)).collect();
    let deck_ids: HashSet<u64> = by_id.keys().copied().collect();
    let scheduler = Fsrs::new(review.retention);
    let now = now_ms();
    // Cards in a region resolved back to the deck (ids absent from the deck —
    // e.g. a topology built before an edit — are skipped).
    let due_of = |ids: &[u64]| {
        let cards: Vec<&Card> = ids.iter().filter_map(|id| by_id.get(id).copied()).collect();
        crate::session::count_reviewable(&cards, store, &scheduler, now, review.retire_after_days)
    };
    // Whole-deck due count: the deck's own cards, plus any of its virtual
    // (remediation) cards that are due — never affecting deck size/composition.
    let deck_due = deck
        .cards
        .iter()
        .filter(|c| {
            crate::session::is_reviewable(c, store, &scheduler, now, review.retire_after_days)
        })
        .count()
        + crate::session::count_reviewable_virtual(
            store,
            &deck.subject,
            &scheduler,
            now,
            review.retire_after_days,
        );
    let topologies = augment
        .topologies_for(&deck_ids)
        .into_iter()
        .map(|t| TopologyInfoDto {
            name: t.name.clone(),
            principle: t.principle.clone(),
            regions: t
                .regions
                .iter()
                .map(|r| RegionInfoDto {
                    name: r.name.clone(),
                    cells: crate::session::card_strengths(&r.cards, store, now),
                    due: due_of(&r.cards),
                })
                .collect(),
        })
        .collect();
    DeckTopologyDto {
        topologies,
        deck_due,
    }
}

/// Serializes a card for the browser, structuring its note via the shared
/// [`render`] model.
fn card_dto(card: &Card) -> CardDto {
    let img_url = |p: &PathBuf| format!("/img/{}", img_key(p));
    CardDto {
        front: card.front.clone(),
        context: card.context.clone(),
        back: card.back_for_display().to_vec(),
        reshaped: card.display_back.is_some(),
        note: render::note_units(card)
            .into_iter()
            .map(NoteUnitDto::from)
            .collect(),
        img: card.image.as_ref().map(img_url),
        img_back: card.image_back.as_ref().map(img_url),
        // The citation is resolved by `review_state`, which has the source base;
        // browse (no base) leaves these empty.
        at: None,
        citation: None,
        citation_error: None,
        // Resolved by `review_state`, which holds the topology; browse and
        // non-topology sessions leave it empty.
        crumb: None,
    }
}

/// A stable, opaque URL key for a resolved image path: the hex `XxHash64` of
/// the path. The card DTO and the image registry derive it the same way, so
/// only paths a deck actually references resolve — no user input is joined to a
/// path, which keeps `/img/` safe from traversal even under `--lan`.
fn img_key(path: &Path) -> String {
    let mut hasher = XxHash64::default();
    hasher.write(path.to_string_lossy().as_bytes());
    format!("{:016x}", hasher.finish())
}

/// Builds the `key → absolute path` registry the `/img/` route serves from, by
/// scanning every card's resolved image sides.
fn collect_images(cards: &[Card]) -> HashMap<String, PathBuf> {
    let mut images = HashMap::new();
    for card in cards {
        for path in [&card.image, &card.image_back].into_iter().flatten() {
            images.insert(img_key(path), path.clone());
        }
    }
    images
}

/// The MIME type to serve a card image with, by file extension. Unknown
/// extensions fall back to a generic binary type (the browser still sniffs it).
fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// The CLI/value name of an input method, matching `Input`'s clap names.
fn input_name(input: Input) -> &'static str {
    match input {
        Input::Type => "type",
        Input::Draw => "draw",
    }
}

/// Parses a grade POST body into a [`Grade`]: either an explicit
/// `{"grade":"failed|partly|passed"}`, or `{"covered":n,"total":m}` from the
/// Explain key-point checklist (derived once, in the lib, via `keypoint_grade`).
fn read_grade(request: &mut Request) -> Option<Grade> {
    #[derive(Deserialize)]
    struct Body {
        grade: Option<String>,
        covered: Option<usize>,
        total: Option<usize>,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    if let Some(g) = body.grade.as_deref() {
        return match g {
            "failed" => Some(Grade::Fail),
            "partly" => Some(Grade::Partial),
            "passed" => Some(Grade::Pass),
            _ => None,
        };
    }
    match (body.covered, body.total) {
        (Some(covered), Some(total)) => Some(keypoint_grade(covered, total)),
        _ => None,
    }
}

/// Parses a `{"index": n}` POST body (the browse card to remove).
fn read_index(request: &mut Request) -> Option<usize> {
    #[derive(Deserialize)]
    struct Body {
        index: usize,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    Some(body.index)
}

fn respond_json<T: Serialize>(request: Request, value: &T) {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    let header = Header::from_bytes(
        &b"Content-Type"[..],
        &b"application/json; charset=utf-8"[..],
    )
    .unwrap();
    let _ = request.respond(Response::from_string(body).with_header(header));
}

fn respond_html(request: Request, html: &str) {
    let header =
        Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
    let _ = request.respond(Response::from_string(html.to_string()).with_header(header));
}

/// Serves a static text asset (the shared `theme.css` / `theme.js`) with the
/// given content type.
fn respond_asset(request: Request, body: &str, content_type: &str) {
    let header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap();
    let _ = request.respond(Response::from_string(body.to_string()).with_header(header));
}

fn respond_status(request: Request, code: u16) {
    let _ = request.respond(Response::from_string(String::new()).with_status_code(code));
}

fn respond_bytes(request: Request, bytes: Vec<u8>, content_type: &str) {
    let header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap();
    let _ = request.respond(Response::from_data(bytes).with_header(header));
}

/// Serves the registered image for `key`, or 404 for an unknown key /
/// unreadable file. Shared by the review and browse routes.
fn serve_image(request: Request, images: &HashMap<String, PathBuf>, key: &str) {
    match images.get(key) {
        Some(path) => match std::fs::read(path) {
            Ok(bytes) => respond_bytes(request, bytes, content_type(path)),
            Err(_) => respond_status(request, 404),
        },
        None => respond_status(request, 404),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn unconfigured_token_leaves_everything_open() {
        assert!(is_authorized("/api/decks", None, None, None));
        assert!(is_authorized("/", None, None, None));
    }

    #[test]
    fn token_guards_only_the_api() {
        let t = Some("secret");
        // open surfaces stay open even with a token set
        assert!(is_authorized("/", None, None, t));
        assert!(is_authorized("/img/deadbeef", None, None, t));
        assert!(is_authorized("/theme.css", None, None, t));
        // /api/* requires the token
        assert!(!is_authorized("/api/decks", None, None, t));
        assert!(!is_authorized("/api/decks", Some("Bearer wrong"), None, t));
        assert!(is_authorized("/api/decks", Some("Bearer secret"), None, t));
        // ?token= query is accepted as a fallback
        assert!(is_authorized("/api/decks", None, Some("secret"), t));
    }

    #[test]
    fn icon_field_registers_an_svg_and_flags_it() {
        let mut icons = HashMap::new();
        let (url, is_svg) = icon_field(Some(Path::new("/ws/assets/icon.svg")), &mut icons);
        let url = url.unwrap();
        assert!(url.starts_with("/img/"));
        assert!(is_svg);
        assert_eq!(icons.len(), 1);
        assert!(icons.values().any(|p| p.ends_with("assets/icon.svg")));

        let (none, flag) = icon_field(None, &mut icons);
        assert!(none.is_none() && !flag);
        assert_eq!(icons.len(), 1);
    }

    #[test]
    fn card_dto_structures_the_note() {
        let note = "Intro here.\n```\nfn main() {}\n```";
        let card = Card::plain(
            Arc::from("s.txt"),
            "the front".to_string(),
            vec!["the back".to_string()],
            Some(note.to_string()),
            1,
        );
        let dto = card_dto(&card);

        assert_eq!(dto.front, "the front");
        assert_eq!(dto.back, vec!["the back".to_string()]);
        assert_eq!(dto.note.len(), 2);
        match &dto.note[0] {
            NoteUnitDto::Sentence { text } => assert_eq!(text, "Intro here."),
            other => panic!("expected a sentence, got {other:?}"),
        }
        match &dto.note[1] {
            NoteUnitDto::Code { lines } => assert_eq!(lines, &vec!["fn main() {}".to_string()]),
            other => panic!("expected a code block, got {other:?}"),
        }
    }

    #[test]
    fn card_dto_exposes_image_urls_and_registry_matches() {
        let mut card = Card::plain(
            Arc::from("s.txt"),
            "q".to_string(),
            vec!["a".to_string()],
            None,
            1,
        );
        card.image = Some(PathBuf::from("/imgs/moon.png"));
        card.image_back = Some(PathBuf::from("/imgs/tab.png"));

        let dto = card_dto(&card);
        let img = dto.img.expect("front image url");
        let img_back = dto.img_back.expect("back image url");
        assert!(img.starts_with("/img/"));
        assert!(img_back.starts_with("/img/") && img_back != img);

        // The registry keys the DTO's URLs derive from, so a request for either
        // URL resolves to the right file.
        let images = collect_images(std::slice::from_ref(&card));
        assert_eq!(
            images.get(img.strip_prefix("/img/").unwrap()),
            Some(&PathBuf::from("/imgs/moon.png"))
        );
        assert_eq!(
            images.get(img_back.strip_prefix("/img/").unwrap()),
            Some(&PathBuf::from("/imgs/tab.png"))
        );
    }

    #[test]
    fn plain_card_has_no_image_urls() {
        let card = Card::plain(
            Arc::from("s.txt"),
            "q".to_string(),
            vec!["a".to_string()],
            None,
            1,
        );
        let dto = card_dto(&card);
        assert!(dto.img.is_none() && dto.img_back.is_none());
        assert!(collect_images(std::slice::from_ref(&card)).is_empty());
    }

    #[test]
    fn content_type_by_extension() {
        assert_eq!(content_type(Path::new("a.png")), "image/png");
        assert_eq!(content_type(Path::new("a.JPG")), "image/jpeg");
        assert_eq!(content_type(Path::new("a.jpeg")), "image/jpeg");
        assert_eq!(content_type(Path::new("a.svg")), "image/svg+xml");
        assert_eq!(content_type(Path::new("a.bin")), "application/octet-stream");
    }

    #[test]
    fn resolve_name_rejects_unknown_deck() {
        let mut known = HashMap::new();
        known.insert("a.txt".to_string(), PathBuf::from("/decks/a.txt"));
        // A known name resolves to its catalog path.
        assert_eq!(
            resolve_name("a.txt", &known),
            Some(PathBuf::from("/decks/a.txt"))
        );
        // An unknown name (e.g. a traversal attempt) resolves to nothing.
        assert_eq!(resolve_name("../etc/passwd", &known), None);
    }

    #[test]
    fn browse_payload_select_phase_has_no_cards() {
        let dto = browse_payload(None);
        assert_eq!(dto.phase, "select");
        assert!(dto.cards.is_empty());
    }

    #[test]
    fn review_state_select_phase_has_no_card() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("p.json")).unwrap();
        let dto = review_state(None, &store);
        assert_eq!(dto.phase, "select");
        assert_eq!(dto.kind, "review");
        assert!(dto.card.is_none());
        // The session-end signal is the `done` phase now, not a `finished` flag:
        // the field is gone from the wire contract entirely.
        let json = serde_json::to_value(&dto).unwrap();
        assert!(json.get("finished").is_none());
    }

    #[test]
    fn finished_review_uses_the_done_phase_not_a_finished_flag() {
        let dir = tempfile::tempdir().unwrap();
        let (mut r, _card, _deck) = one_card_reviewing(dir.path());
        let mut store = Store::open(dir.path().join("graded.json")).unwrap();
        // Pass the only card → the queue empties → the session is finished.
        r.session.grade(&mut store, Grade::Pass, now_ms());
        assert!(r.session.is_finished());
        let dto = review_state(Some(&r), &store);
        assert_eq!(dto.phase, "done");
        assert_eq!(dto.kind, "review");
    }

    #[test]
    fn grade_names_map_to_grades() {
        // A guard so the JSON contract and the Grade enum stay in sync.
        assert!(matches!(Grade::Fail, Grade::Fail));
        assert_eq!(mode_name(Mode::LineByLine), "line");
        assert_eq!(mode_name(Mode::Flip), "flip");
        assert_eq!(mode_name(Mode::Explain), "explain");
    }

    #[test]
    fn input_name_matches_clap_value_names() {
        assert_eq!(input_name(Input::Type), "type");
        assert_eq!(input_name(Input::Draw), "draw");
    }

    // ---- ask-Claude server state machine -------------------------------
    //
    // These drive `poll_ask` through a channel we control, so the actual CLI
    // execution (covered by `ask.rs`'s own tests) isn't involved.

    fn one_card_reviewing(dir: &Path) -> (Reviewing, Card, PathBuf) {
        let deck = dir.join("d.txt");
        std::fs::write(&deck, "# front\n\tback\n").unwrap();
        let store = Store::open(dir.join("p.json")).unwrap();
        let card = Card::plain(
            Arc::from("d.txt"),
            "front".to_string(),
            vec!["back".to_string()],
            None,
            1,
        );
        let session = Session::new(
            vec![card.clone()],
            &store,
            Box::new(Fsrs::default()),
            crate::session::SessionOptions::default(),
            now_ms(),
        );
        let mut decks = HashMap::new();
        decks.insert("d.txt".to_string(), deck.clone());
        let reviewing = Reviewing::new(SessionBuild {
            session,
            label: "d.txt".to_string(),
            decks,
            links: HashMap::new(),
            source_roots: HashMap::new(),
            source_bases: HashMap::new(),
            topology_name: None,
        });
        (reviewing, card, deck)
    }

    #[test]
    fn poll_ask_records_answer_in_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let (mut r, card, _deck) = one_card_reviewing(dir.path());
        let (tx, rx) = std::sync::mpsc::channel();
        r.ask.pending = Some(Pending {
            rx,
            purpose: Purpose::Question("why is s1 invalid?".to_string()),
            card,
        });
        // Nothing delivered yet: still thinking, no-op poll.
        assert_eq!((None, None), r.poll_ask());
        assert!(r.ask_dto(None, None).thinking);

        tx.send(Reply::Answer("because ownership moved".to_string()))
            .unwrap();
        assert_eq!((None, None), r.poll_ask());
        assert!(r.ask.pending.is_none());
        assert_eq!(1, r.ask.transcript.len());
        assert_eq!("why is s1 invalid?", r.ask.transcript[0].0);
        assert_eq!("because ownership moved", r.ask.transcript[0].1);
        assert!(r.ask.cli.started); // later questions --resume
    }

    #[test]
    fn ask_transcript_resets_when_the_card_changes() {
        let dir = tempfile::tempdir().unwrap();
        let (mut r, card, _deck) = one_card_reviewing(dir.path());
        // A previous card's discussion is on display, and the conversation has
        // begun (the CLI session is live).
        r.ask
            .transcript
            .push(("old q".to_string(), "old a".to_string()));
        r.ask.subject = Some(card.id().wrapping_add(1)); // a different card
        r.ask.cli.started = true;

        r.align_transcript();

        // The current card differs from the transcript's card, so the display is
        // cleared and re-tagged — but Claude's conversation context survives.
        assert!(r.ask.transcript.is_empty());
        assert_eq!(Some(card.id()), r.ask.subject);
        assert!(r.ask.cli.started);
    }

    #[test]
    fn poll_ask_condense_appends_note_to_deck() {
        let dir = tempfile::tempdir().unwrap();
        let (mut r, card, deck) = one_card_reviewing(dir.path());
        r.ask.transcript.push(("q".to_string(), "a".to_string()));
        let (tx, rx) = std::sync::mpsc::channel();
        r.ask.pending = Some(Pending {
            rx,
            purpose: Purpose::Condense,
            card,
        });
        tx.send(Reply::Answer("- key insight to reread".to_string()))
            .unwrap();
        let (status, error) = r.poll_ask();
        assert_eq!(Some("note saved".to_string()), status);
        assert!(error.is_none());
        let text = std::fs::read_to_string(&deck).unwrap();
        assert!(text.contains("key insight to reread"), "deck:\n{text}");
    }

    #[test]
    fn poll_ask_error_resets_session() {
        let dir = tempfile::tempdir().unwrap();
        let (mut r, card, _deck) = one_card_reviewing(dir.path());
        r.ask.cli.started = true;
        let (tx, rx) = std::sync::mpsc::channel();
        r.ask.pending = Some(Pending {
            rx,
            purpose: Purpose::Question("q".to_string()),
            card,
        });
        tx.send(Reply::Error("not logged in".to_string())).unwrap();
        let (status, error) = r.poll_ask();
        assert_eq!(Some("not logged in".to_string()), error);
        assert!(status.is_none());
        assert!(r.ask.pending.is_none());
        assert!(!r.ask.cli.started); // a fresh session next time
        assert!(r.ask.transcript.is_empty());
    }

    // ── trace walk ──────────────────────────────────────────────────────

    /// A two-checkpoint trace over a single source file, in `dir`.
    fn walk_deck(dir: &Path) -> crate::trace::Trace {
        std::fs::write(dir.join("source.txt"), "first\nsecond\nthird\n").unwrap();
        let path = dir.join("t.txt");
        std::fs::write(
            &path,
            "% trace: how it works\n\
             % source: source.txt\n\
             # Predict the first hop\n\
             \t% given: line — the input line\n\
             \tit reads the first line\n\
             \t% at: 1\n\
             # Predict the second hop\n\
             \tit reads line two\n\
             \t% at: 2\n",
        )
        .unwrap();
        crate::trace::Trace::from_deck(&Deck::load(&path).unwrap()).unwrap()
    }

    #[test]
    fn walk_dto_tracks_phase_excerpt_and_rail() {
        let dir = tempfile::tempdir().unwrap();
        let trace = walk_deck(dir.path());
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let walk = Walk::new(trace);
        let mut w = Walking::new(walk, None);

        // Predict: prompt + givens, no excerpt yet, the first node is current.
        let d = walk_dto(&w);
        assert_eq!("walk", d.kind);
        assert_eq!("predict", d.phase);
        assert_eq!(1, d.current);
        assert_eq!(2, d.total);
        assert_eq!(Some("Predict the first hop".to_string()), d.prompt);
        assert_eq!(vec!["line — the input line".to_string()], d.givens);
        assert!(d.excerpt.is_none());
        assert!(!d.auto_grade);
        assert!(d.path[0].current && d.path[0].delta.is_none());

        // Reveal: the live excerpt is read, the prediction is recalled.
        w.walk.predict("my guess".to_string());
        let d = walk_dto(&w);
        assert_eq!("reveal", d.phase);
        assert_eq!(Some("my guess".to_string()), d.prediction);
        let ex = d.excerpt.expect("reveal reads the source");
        assert_eq!(
            vec![(1, "first".to_string())],
            ex.lines
                .iter()
                .map(|l| (l.n, l.text.clone()))
                .collect::<Vec<_>>()
        );
        assert_eq!(vec!["it reads the first line".to_string()], d.points);

        // Grade Got: the rail colors the walked node and advances to hop 2.
        w.walk.grade(&mut store, Delta::Passed, 1000);
        let d = walk_dto(&w);
        assert_eq!("predict", d.phase);
        assert_eq!(2, d.current);
        assert_eq!(Some("passed"), d.path[0].delta);
        assert!(d.path[1].current);

        // Walk the last hop → done with a summary (the drill; verification is the
        // separate trace exam, not an in-walk compression).
        w.walk.predict(String::new());
        w.walk.grade(&mut store, Delta::Failed, 1001);
        let d = walk_dto(&w);
        assert_eq!("done", d.phase);
        let s = d.summary.expect("done has a summary");
        assert_eq!((1, 0, 1), (s.passed, s.partly, s.failed));
        assert_eq!(vec![2], s.weak); // 1-based: the failed second hop
    }

    #[test]
    fn walk_dto_surfaces_a_live_grade_and_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let trace = walk_deck(dir.path());
        let walk = Walk::new(trace);
        let mut w = Walking::new(walk, Some(AskConfig::default()));

        w.walk.predict("g".to_string());
        // Simulate the background grade resolving (no real CLI call in the test).
        w.grade_result = Some((Delta::Partial, "right idea, missed a detail".to_string()));
        let d = walk_dto(&w);
        assert!(d.auto_grade);
        assert_eq!(Some("Partly"), d.verdict);
        assert_eq!(Some("right idea, missed a detail".to_string()), d.feedback);

        w.clear_grade();
        let d = walk_dto(&w);
        assert!(d.verdict.is_none() && d.feedback.is_none() && !d.thinking);
    }

    #[test]
    fn walk_ask_condense_appends_a_note_to_the_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let trace = walk_deck(dir.path());
        let deck_path = trace.deck_path.clone();
        let walk = Walk::new(trace);
        let mut w = Walking::new(walk, None);
        w.walk.predict("guess".to_string()); // reveal hop 1 (a current checkpoint)

        // A condense reply is in flight (no real CLI call), about the synthesized
        // checkpoint card — its line points at the checkpoint in the deck file.
        let card = w.checkpoint_card().expect("a checkpoint card");
        let (tx, rx) = std::sync::mpsc::channel();
        w.ask.pending = Some(Pending {
            rx,
            purpose: Purpose::Condense,
            card,
        });
        tx.send(Reply::Answer(
            "- the read lock is released first".to_string(),
        ))
        .unwrap();

        let (status, error) = w.poll_ask();
        assert_eq!(Some("note saved".to_string()), status);
        assert!(error.is_none());
        let text = std::fs::read_to_string(&deck_path).unwrap();
        assert!(
            text.contains("the read lock is released first"),
            "deck:\n{text}"
        );
    }

    // ── Augment screen (the picker's "Augment" action) ──

    fn aug_card(front: &str, back: &str) -> Card {
        Card::plain(
            Arc::from("d.txt"),
            front.to_string(),
            vec![back.to_string()],
            None,
            1,
        )
    }

    #[test]
    fn augmenting_reports_coverage_and_removal_persists() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("augment.json");
        let cards = vec![aug_card("Q1", "a"), aug_card("Q2", "b")];

        // Seed the on-disk cache: one card has distractors, the other a note.
        let mut seed = AugmentCache::open(&cache_path);
        seed.set_distractors(cards[0].id(), vec!["x".into()]);
        seed.set_note(cards[1].id(), "n".into());
        seed.save().unwrap();

        let mut aug = Augmenting::open("d.txt".into(), cards.clone(), cache_path.clone());
        let dto = aug.dto();
        assert_eq!(2, dto.cards);
        assert!(dto.busy.is_none());
        let choices = dto.rows.iter().find(|r| r.kind == "choices").unwrap();
        assert_eq!((1, 2), (choices.covered, choices.eligible));
        let topo = dto.rows.iter().find(|r| r.kind == "topology").unwrap();
        assert!(topo.items.is_empty());

        // Removing a target writes through to disk; other targets are untouched.
        assert!(aug.remove("choices", None));
        assert_eq!(
            0,
            aug.dto()
                .rows
                .iter()
                .find(|r| r.kind == "choices")
                .unwrap()
                .covered
        );
        let reloaded = AugmentCache::open(&cache_path);
        assert_eq!(None, reloaded.distractors(cards[0].id()));
        assert_eq!(Some("n"), reloaded.note(cards[1].id()));

        assert!(!aug.remove("bogus", None)); // unknown target → no-op
    }

    #[test]
    fn augmenting_generate_is_a_noop_when_a_target_is_fully_covered() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("augment.json");
        let cards = vec![aug_card("Q", "a")];

        let mut seed = AugmentCache::open(&cache_path);
        seed.set_distractors(cards[0].id(), vec!["x".into()]);
        seed.save().unwrap();

        let mut aug = Augmenting::open("d.txt".into(), cards, cache_path);
        // Fully covered → no gap → no costed call is started.
        let started = aug.generate("choices", None, &AiConfig::default(), &AskConfig::default());
        assert!(!started);
        assert!(aug.dto().busy.is_none());
    }

    #[test]
    fn deck_topology_dto_deck_due_includes_a_due_virtual_card() {
        let dir = tempfile::tempdir().unwrap();
        let deck_path = dir.path().join("rust.txt");
        std::fs::write(&deck_path, "# q1\n\ta1\n").unwrap();
        let deck = Deck::load(&deck_path).unwrap();

        let mut store = Store::open(dir.path().join("progress.json")).unwrap();
        let now = now_ms();
        // The one deck card has graduated and isn't due — no deck contribution.
        store.get_or_insert(deck.cards[0].id(), now).fsrs = Some(crate::store::FsrsState {
            state: 2,
            scheduled_days: 30,
            due_ms: now + 30 * 86_400_000,
            ..Default::default()
        });
        let augment = AugmentCache::open(augment::augment_path_for(store.path()));

        let before = deck_topology_dto(&augment, &store, &deck, ReviewConfig::default());
        assert_eq!(0, before.deck_due);

        // A due virtual card for this deck adds to the whole-deck due count —
        // sidecar content keyed by its `Card::id`, plus a fresh schedule at t=0.
        let vtext = "# virtual front\n\tvirtual back\n".to_string();
        let vid = crate::parser::parse_str(&deck.subject, &vtext).unwrap()[0].id();
        store.insert_virtual(crate::store::VirtualCard {
            id: vid,
            kind: crate::store::VirtualKind::Remediation,
            parent: deck.subject.clone(),
            text: vtext,
            created_ms: 0,
        });
        store.get_or_insert(vid, 0);

        let after = deck_topology_dto(&augment, &store, &deck, ReviewConfig::default());
        assert_eq!(1, after.deck_due);
    }
}
