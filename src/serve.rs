//! A local web frontend.
//!
//! Bare `alix` starts a small synchronous HTTP server (one request at a
//! time — correct for a single user) that serves an embedded web page and a
//! JSON API — the sole interactive frontend. The [`Session`]/[`Store`] drive
//! review, and cards are sent to the browser as a DTO built from
//! [`render::note_units`], so the note structuring lives in one place. Grades
//! persist to the same progress store the rest of the CLI (`deck`, `trace`,
//! `generate`, …) reads and writes, so studying in the browser and running
//! those commands share one history.
//!
//! It is deliberately local-only: no accounts, no database. By default it
//! binds to `127.0.0.1`; `--lan` binds all interfaces so a phone or tablet on
//! the same network can reach it (there is no authentication, so that is
//! opt-in).

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    hash::Hasher,
    io::Read,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
    time::Instant,
};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server};
use twox_hash::XxHash64;

use crate::{
    answer::{Input, Mode, TypedResult, grade_lines_ordered, grade_lines_unordered, mode_name},
    ask::{self, CliSession, Exchange, Reply},
    augment::{self, AugmentCache},
    card::Card,
    choice::{self, ChoiceQuestion},
    config::{
        AiConfig, AskConfig, Bindings, BrowseBindings, Config, ExamConfig, GenerateDeckConfig, Key,
        KeyPattern, PickerKeys, ReviewConfig, Strictness,
    },
    deck::{self, Deck, DeckState},
    depth::{self, Depth, Reveal, depth_name},
    doctor, exam, generate, import, picker,
    recent::RecentDecks,
    render::{self, NoteUnit},
    scheduler::{Fsrs, Grade, keypoint_grade},
    session::{Session, now_ms},
    share,
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

// ── Wire contract ────────────────────────────────────────────────────────────
// The DTOs below are the client-agnostic JSON contract (docs/API.md), pinned
// by `mod contract` in this file's tests. Change code, doc, and CHANGELOG
// together; tests/contracts/*.json is the generated codegen corpus.

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
    /// The answer mode name (`flip`, `line`, `typeline`, …); the page reveals
    /// line-by-line for `line` and flip-style otherwise. Derived from the card's
    /// `% reveal:` and the session's `depth`.
    mode: &'static str,
    /// The session's chosen depth (`recognize` / `recall` / `reconstruct`) — the
    /// depth of practice this session runs at (spec §4).
    depth: &'static str,
    /// The input method (`type` / `draw`). `draw` tells the page to show the
    /// canvas for a self-graded card; orthogonal to `mode`. The runtime "Draw
    /// answers" toggle lives in the browser and never appears here.
    input: &'static str,
    remaining: usize,
    initial: usize,
    reviews: usize,
    passed: usize,
    failed: usize,
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

/// The deck-selection catalog sent to the browser picker, in three sections:
/// `workspaces` (each with its last-progress time), `recent` loose decks
/// (recent-first), and plain `folders`. A deck inside a
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
/// 🕒, nothing due), `mastered` (🎉, tucked into the Mastered window).
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
    /// Any card hasn't yet been correctly picked at Recognize — see
    /// [`picker::DeckStatus::reviewable_recognize`].
    reviewable_recognize: bool,
    /// A card is due (or fresh), or a virtual card is due, at Recall — see
    /// [`picker::DeckStatus::reviewable_recall`].
    reviewable_recall: bool,
    /// A card is due at Reconstruct — see
    /// [`picker::DeckStatus::reviewable_reconstruct`].
    reviewable_reconstruct: bool,
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
    /// The highest depth with a badge to show (`"reconstruct"` / `"recall"` /
    /// `"recognize"`), or `null` for none yet — see [`picker::DeckStatus::badge_depth`].
    /// Additive telemetry; gates nothing.
    badge_depth: Option<&'static str>,
    /// `true` when `badge_depth`'s badge lapsed (earned once, not currently
    /// solid) and should render dotted rather than solid.
    badge_dotted: bool,
    /// Any deck card has no store entry yet — fresh material, distinct from
    /// `state`/`meta`.
    new_cards: bool,
    /// The learner's last-used session depth for this deck (`"recognize"` /
    /// `"recall"` / `"reconstruct"`), defaulting to `"recall"`.
    last_depth: &'static str,
}

/// A workspace member deck in the drill-in list: a qualified selection `name`
/// (`<workspace>/<file>`), its display `label` and status (badge/state/locked/
/// reviewable/mastered/trace, from the workspace's own store), and its `indent`
/// in the unlock dependency tree (0 = a foundation root).
#[derive(Debug, Serialize)]
struct MemberDto {
    name: String,
    label: String,
    meta: Option<String>,
    state: &'static str,
    locked: bool,
    reviewable: bool,
    /// Per-depth due-ness — see [`DeckItemDto::reviewable_recognize`] /
    /// [`DeckItemDto::reviewable_recall`] / [`DeckItemDto::reviewable_reconstruct`].
    reviewable_recognize: bool,
    reviewable_recall: bool,
    reviewable_reconstruct: bool,
    mastered: bool,
    is_trace: bool,
    examable: bool,
    has_exam: bool,
    indent: usize,
    /// The `├─`/`└─`/`│` tree-branch prefix drawn before the label, so a
    /// member's dependency chain reads as a tree, not just indentation.
    tree: String,
    /// `true` when the member deck has a cached topology (a focus drawer).
    has_topology: bool,
    /// The highest depth with a badge to show, from the workspace's own store —
    /// same semantics as [`DeckItemDto::badge_depth`].
    badge_depth: Option<&'static str>,
    /// `true` when `badge_depth`'s badge lapsed (renders dotted, not solid).
    badge_dotted: bool,
    /// Any member-deck card has no store entry yet — fresh material.
    new_cards: bool,
    /// The learner's last-used session depth for this member deck, defaulting
    /// to `"recall"` — what a plain Learn launches.
    last_depth: &'static str,
}

/// The result of answering a choice card: which option was picked, which is
/// correct, and whether they match. The page highlights the options with this.
#[derive(Debug, Serialize)]
struct ChooseFeedbackDto {
    chosen: usize,
    correct: usize,
    passed: bool,
}

/// The result of submitting a typed answer: an honest `{ input, expected,
/// passed }` per back line ([`TypedResult`]) plus whether they all passed. Pure
/// evidence — the grade is the learner's, applied separately via `/api/grade`.
#[derive(Debug, Serialize)]
struct CheckFeedbackDto {
    results: Vec<TypedResult>,
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

/// The web doctor report (`GET /api/doctor`) — the CLI's free checks,
/// serialized. Costed backend probes stay CLI-only.
#[derive(Serialize)]
struct DoctorDto {
    rows: Vec<DoctorRowDto>,
}

#[derive(Serialize)]
struct DoctorRowDto {
    name: &'static str,
    /// `ok` | `warn` | `fail` (open set).
    status: &'static str,
    detail: String,
    remedy: Option<String>,
}

impl From<doctor::Finding> for DoctorRowDto {
    fn from(f: doctor::Finding) -> Self {
        DoctorRowDto {
            name: f.name,
            status: match f.status {
                doctor::Status::Ok => "ok",
                doctor::Status::Warn => "warn",
                doctor::Status::Fail => "fail",
            },
            detail: f.detail,
            remedy: f.remedy,
        }
    }
}

/// The pairing sheet (`GET /api/pair`): the URL another device should open,
/// and a server-rendered QR of it.
#[derive(Serialize)]
struct PairDto {
    url: String,
    /// A server-rendered QR of `url`; `null` when this is a localhost-only
    /// instance (nothing another device could reach).
    svg: Option<String>,
    lan: bool,
}

/// The result of `POST /api/reset`: the row's resolved display name and how
/// many card schedules it wiped.
#[derive(Serialize)]
struct ResetDto {
    deck: String,
    cards_cleared: usize,
}

/// The result of `POST /api/import`: the placed file's name and its card
/// count. Unlike `generate`'s lenient save, an upload that doesn't parse is
/// rejected outright — see the `/api/import` handler.
#[derive(Serialize)]
struct ImportDto {
    deck: String,
    cards: usize,
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
/// filter, open the Mastered window, and the depth menu (open it + start at
/// one of the three depths). (Jump to first/last is fixed at `g`/`G`/Home/End
/// on the page, so it isn't sent.)
#[derive(Debug, Serialize)]
struct PickerKeysDto {
    up: Vec<KeyDto>,
    down: Vec<KeyDto>,
    open: Vec<KeyDto>,
    back: Vec<KeyDto>,
    filter: Vec<KeyDto>,
    mastered: Vec<KeyDto>,
    depth: Vec<KeyDto>,
    recognize: Vec<KeyDto>,
    recall: Vec<KeyDto>,
    reconstruct: Vec<KeyDto>,
    cram: Vec<KeyDto>,
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
            depth: key_list(&k.depth),
            recognize: key_list(&k.recognize),
            recall: key_list(&k.recall),
            reconstruct: key_list(&k.reconstruct),
            cram: key_list(&k.cram),
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
    /// Ask-Claude settings (command, allowlist, timeout, …).
    pub ask: AskConfig,
    /// AI-exam settings (model, question count, default strictness, …).
    pub exam: ExamConfig,
    /// AI augmentation settings (model, per-target counts), for generating
    /// augmentations from the picker's Augment screen.
    pub ai: AiConfig,
    /// AI deck-generation settings (model, timeout, max cards, guidance, …),
    /// for `POST /api/generate`.
    pub generate: GenerateDeckConfig,
    /// Personal review pacing (FSRS retention + retirement interval), for the
    /// selection screen's badges and due counts.
    pub review: ReviewConfig,
    /// Pairing token required on `/api/*` when set (auto-generated for `--lan`);
    /// `None` leaves the server open (the localhost default).
    pub auth: Option<String>,
    /// The `--config` path the launcher loaded config from (`None` → the
    /// platform default), passed through so `/api/doctor` checks the same file.
    pub config_path: Option<PathBuf>,
    /// How this instance is reached, for `/api/pair`'s pairing sheet.
    pub pair: PairInfo,
    /// `true` for a scoped `alix <dir>` launch — its decks dir is pinned to
    /// that folder forever. `false` for the config-derived launch (bare
    /// `alix`), whose `/api/decks` re-resolves the configured dir on every
    /// fetch so a `decks_dir` edit takes effect without a restart.
    pub scoped: bool,
}

/// How this instance is reached, for the pairing sheet. Built by the
/// launcher, which is the only place that knows bind + token + LAN IP.
pub struct PairInfo {
    pub url: String,
    pub lan: bool,
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
    /// AI-grades each prediction when set (`[trace] auto_grade` + the ask
    /// config); `None` = self-graded.
    pub grade: Option<AskConfig>,
}

/// A browse card list ready to serve, with its label and deck paths.
pub struct CardsBuild {
    pub cards: Vec<Card>,
    pub label: String,
    pub decks: HashMap<String, PathBuf>,
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

    /// Completes a question with `answer` right away, without spawning the
    /// backend or ever creating a [`Pending`] — used when serve already knows,
    /// at prompt-build time, that the model would only be asked to echo
    /// [`ask::SOURCE_NOT_FOUND`] verbatim (`{#source-not-found-reply}`: a frozen
    /// card whose live source root can't be resolved). Applies the same
    /// subject-alignment and transcript push as [`Ask::poll`]'s
    /// `(Reply::Answer, Purpose::Question)` arm, so the resulting `AskDto` is
    /// indistinguishable from a real reply — except it's already there,
    /// `thinking: false`, on the very next read. Unlike `poll`, it leaves
    /// `self.cli` untouched: no real turn happened, so a later real question
    /// still starts/resumes the CLI session correctly. Returns `false` (no-op)
    /// if a call is already pending.
    fn answer_immediately(&mut self, card: &Card, question: String, answer: String) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.align(Some(card.id()));
        // `self.cli` is deliberately untouched: no real turn happened, so a
        // later question must still start/resume the CLI session correctly.
        self.transcript.push((question, answer));
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
        // `{#source-not-found-reply}`: this is exactly the condition under which
        // `ask::question_context`'s `(Some(excerpt), None)` arm would tell the
        // model to reply `SOURCE_NOT_FOUND` verbatim — a round trip that spends
        // real latency (and a chance of the model paraphrasing) to echo a
        // constant. Answer it here instead: deterministic wording, zero cost.
        if let Some(q) = &question
            && frozen.is_some()
            && live_root.is_none()
        {
            return self.ask.answer_immediately(
                &card,
                q.clone(),
                ask::SOURCE_NOT_FOUND.to_string(),
            );
        }
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
    /// Milliseconds until a failed trace exam may be re-sat — set only in the
    /// `cooldown` phase, `null` everywhere else.
    cooldown_ms: Option<u64>,
}

/// The `phase:"cooldown"` ExamDto for a trace exam still cooling down after a
/// fail: no sitting exists, so every session field is at its baseline and only
/// `deck` + `cooldown_ms` carry information.
fn cooldown_dto(deck: &str, cooldown_ms: u64) -> ExamDto {
    ExamDto {
        phase: "cooldown",
        deck: deck.to_string(),
        strictness: "balanced",
        total: 0,
        current: 0,
        question: None,
        answer: String::new(),
        on_last: false,
        grades: Vec::new(),
        passed: None,
        gaps: Vec::new(),
        can_remediate: false,
        remediated_count: None,
        is_trace: true,
        unlocks: Vec::new(),
        thinking: false,
        error: None,
        elapsed: None,
        cooldown_ms: Some(cooldown_ms),
    }
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
        cooldown_ms: None,
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

/// A deck generation in flight (or just finished): the worker channel, what
/// was asked, where the deck lands, and the outcome once placed.
struct Generating {
    rx: Receiver<Result<String, String>>,
    url: String,
    dest: PathBuf,
    started: Instant,
    outcome: Option<Result<(String, usize), String>>,
}

#[derive(Serialize)]
struct GenerateDto {
    /// `generating` | `done` | `error` (open set).
    phase: &'static str,
    deck: Option<String>,
    cards: Option<usize>,
    elapsed: Option<u64>,
    error: Option<String>,
}

impl Generating {
    fn dto(&self) -> GenerateDto {
        match &self.outcome {
            None => GenerateDto {
                phase: "generating",
                deck: None,
                cards: None,
                elapsed: Some(self.started.elapsed().as_secs()),
                error: None,
            },
            Some(Ok((deck, cards))) => GenerateDto {
                phase: "done",
                deck: Some(deck.clone()),
                cards: Some(*cards),
                elapsed: Some(self.started.elapsed().as_secs()),
                error: None,
            },
            Some(Err(e)) => GenerateDto {
                phase: "error",
                deck: None,
                cards: None,
                elapsed: Some(self.started.elapsed().as_secs()),
                error: Some(e.clone()),
            },
        }
    }

    /// Drains a finished worker and places the deck (lenient, like the CLI:
    /// a parse problem still saves the file and is reported as the error).
    fn poll(&mut self) {
        if self.outcome.is_some() {
            return;
        }
        let text = match self.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.outcome = Some(Err("the generate worker exited unexpectedly".to_string()));
                return;
            }
        };
        self.outcome = Some(text.and_then(|t| {
            let name = generate::deck_name(&self.url);
            match crate::library::place_deck(&self.dest, &name, &t) {
                Ok(p) => {
                    let deck = p
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    match p.parse_error {
                        None => Ok((deck, p.cards)),
                        Some(e) => Err(format!("saved {deck}, but it does not parse yet: {e}")),
                    }
                }
                Err(e) => Err(format!("{e:#}")),
            }
        }));
    }
}

/// A wormhole send in flight: the staged copy (kept alive for the whole
/// transfer), the job, and what it has reported so far.
struct Sharing {
    job: share::ShareJob,
    _stage: tempfile::TempDir,
    code: Option<String>,
    started: Instant,
    outcome: Option<Result<(), String>>,
}

#[derive(Serialize)]
struct ShareDto {
    /// `staging` | `code` | `sent` | `error` (open set).
    phase: &'static str,
    code: Option<String>,
    elapsed: Option<u64>,
    error: Option<String>,
}

impl Sharing {
    fn poll(&mut self) {
        while let Ok(ev) = self.job.events.try_recv() {
            match ev {
                share::ShareEvent::Code(c) => self.code = Some(c),
                share::ShareEvent::Done => self.outcome = Some(Ok(())),
                share::ShareEvent::Error(e) => self.outcome = Some(Err(e)),
            }
        }
    }

    fn dto(&self) -> ShareDto {
        let elapsed = Some(self.started.elapsed().as_secs());
        match (&self.outcome, &self.code) {
            (Some(Err(e)), _) => ShareDto {
                phase: "error",
                code: self.code.clone(),
                elapsed,
                error: Some(e.clone()),
            },
            (Some(Ok(())), _) => ShareDto {
                phase: "sent",
                code: self.code.clone(),
                elapsed,
                error: None,
            },
            (None, Some(_)) => ShareDto {
                phase: "code",
                code: self.code.clone(),
                elapsed,
                error: None,
            },
            (None, None) => ShareDto {
                phase: "staging",
                code: None,
                elapsed,
                error: None,
            },
        }
    }
}

/// Stages a row for sharing into `tmp`: a deck file travels as-is (its
/// augmentations live in the store-side cache and stay home); a folder is
/// copied minus personal state. Returns what to hand to wormhole/zip.
fn stage_for_share(path: &Path, tmp: &tempfile::TempDir) -> Result<PathBuf> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    if !crate::workspace::has_decks(path) {
        bail!("no decks in `{}` — nothing to share", path.display());
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("shared-decks");
    let stage = tmp.path().join(name);
    share::stage_dir(path, &stage)?;
    Ok(stage)
}

/// A wormhole receive in flight: the scratch dir it lands in, where it goes
/// afterwards, and the landing outcome.
struct Receiving {
    job: share::ShareJob,
    tmp: tempfile::TempDir,
    dest: PathBuf,
    started: Instant,
    outcome: Option<Result<(String, Vec<String>), String>>,
}

#[derive(Serialize)]
struct ReceiveDto {
    /// `receiving` | `done` | `error` (open set).
    phase: &'static str,
    landed: Option<String>,
    stripped: Vec<String>,
    elapsed: Option<u64>,
    error: Option<String>,
}

impl Receiving {
    fn poll(&mut self) {
        if self.outcome.is_some() {
            return;
        }
        while let Ok(ev) = self.job.events.try_recv() {
            match ev {
                share::ShareEvent::Code(_) => {} // receive never emits one
                share::ShareEvent::Done => {
                    // `land_received`'s collision check is check-then-act;
                    // safe only because this server loop is single-threaded
                    // (one request handled at a time, and `poll()` only ever
                    // runs from inside that loop) — introduce no threads here.
                    self.outcome = Some(
                        share::land_received(self.tmp.path(), &self.dest)
                            .map_err(|e| format!("{e:#}")),
                    );
                }
                share::ShareEvent::Error(e) => self.outcome = Some(Err(e)),
            }
        }
    }

    fn dto(&self) -> ReceiveDto {
        let elapsed = Some(self.started.elapsed().as_secs());
        match &self.outcome {
            None => ReceiveDto {
                phase: "receiving",
                landed: None,
                stripped: Vec::new(),
                elapsed,
                error: None,
            },
            Some(Ok((landed, stripped))) => ReceiveDto {
                phase: "done",
                landed: Some(landed.clone()),
                stripped: stripped.clone(),
                elapsed,
                error: None,
            },
            Some(Err(e)) => ReceiveDto {
                phase: "error",
                landed: None,
                stripped: Vec::new(),
                elapsed,
                error: Some(e.clone()),
            },
        }
    }
}

/// Serves review on the already-bound `server` until the process is stopped
/// (binding happens at the call site, *before* the URL is announced — so a
/// port clash errors before any success-looking output), opening on the
/// in-browser deck-selection screen; picking decks (`POST /api/select`)
/// calls `build` to construct a session in place.
/// `build` borrows the shared `store` and `recent`, so all sessions write one
/// history and update the recent-decks list, exactly like the CLI.
/// Binds the server socket — separated from [`run_review`] so a port clash
/// errors before the caller announces a URL, and with the multi-instance
/// remedy in the message.
pub fn bind(addr: SocketAddr) -> Result<Server> {
    Server::http(addr).map_err(|e| {
        anyhow!(
            "cannot start the server on {addr}: {e} — is another alix using this port? try --port"
        )
    })
}

#[expect(clippy::too_many_arguments)] // each is a distinct, named server input
pub fn run_review(
    mut store: Store,
    mut recent: RecentDecks,
    mut decks_dir: PathBuf,
    server: Server,
    opts: ReviewOptions,
    mut build: impl FnMut(
        Vec<PathBuf>,
        &SelectOptions,
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
        ask: ask_cfg,
        exam: exam_cfg,
        ai: ai_cfg,
        generate: generate_cfg,
        review: review_cfg,
        auth,
        config_path,
        pair,
        scoped,
    } = opts;
    let keys = ReviewKeys::from(&bindings);
    let picker_keys = PickerKeysDto::from(&picker_keys);
    // The `/browse` page this server also hosts needs its own next/prev/remove
    // keys, distinct from the review grade keys served at `/api/keys`.
    let browse_keys = BrowseKeys::from(&browse_bindings);
    let ask_info = AskInfoDto::from(&ask_cfg);
    // The server always opens on the picker; review/browse states are entered
    // from it (`/api/select`, `/api/browse`) — browse is a native mode of the
    // review server, not a separate page.
    let (mut reviewing, mut browsing): (Option<Reviewing>, Option<Browsing>) = (None, None);
    let mut examining: Option<Examining> = None;
    // The picker's "Augment" action opens a deck's augmentation screen here.
    let mut augmenting: Option<Augmenting> = None;
    // The add-sheet's "Generate from URL" action; one deck generation at a time.
    let mut generating: Option<Generating> = None;
    // The picker's "Share" action; one wormhole send in flight at a time. An
    // abandoned/replaced job always drops through `Sharing`/`ShareJob` (never
    // leaked), so its child process is cancelled even without a close call.
    let mut sharing: Option<Sharing> = None;
    // The picker's "Receive" action; one wormhole receive in flight at a time.
    // Same drop-cancels invariant as `sharing` — an abandoned/replaced job
    // always drops through `ShareJob`, cancelling its wormhole child.
    let mut receiving: Option<Receiving> = None;
    // A trace picked from the selection screen walks in-page inside review.html
    // (no navigation to a separate `/walk` page — the walk is an in-page mode).
    let mut walking: Option<Walking> = None;
    // `browsing` (seeded above for a `--serve` browse launch) is also entered
    // from the picker's "Browse" action (POST /api/browse) — in-page, no page nav.
    // Workspace icons resolved while building the picker, served via `/img/` at
    // launcher time (when no review/browse session owns the registry).
    let mut launcher_icons: HashMap<String, PathBuf> = HashMap::new();
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
            (Method::Get, "/api/doctor") => {
                let (cfg, _) = doctor::check_config(config_path.as_deref());
                let rows = vec![
                    cfg,
                    doctor::check_store(Some(store.path().to_path_buf())),
                    doctor::check_decks(&decks_dir),
                    // Mirrors `main.rs::doctor_cmd`'s binary lines verbatim
                    // (names, purposes, remedies) — the web report must match
                    // the CLI's, minus the costed `--backends` probe.
                    doctor::check_binary(
                        "backend",
                        &ask_cfg.command,
                        "the AI features (tutor, exam, generate)",
                        "install it and log in — or switch `[ask] backend` in the config",
                    ),
                    doctor::check_binary(
                        "share",
                        "wormhole",
                        "sharing (`alix share`/`receive`)",
                        "install magic-wormhole (e.g. `pipx install magic-wormhole`, or your package manager)",
                    ),
                ]
                .into_iter()
                .map(DoctorRowDto::from)
                .collect();
                respond_json(request, &DoctorDto { rows })
            }
            (Method::Get, "/api/pair") => {
                let svg = if pair.lan {
                    crate::qr::svg(&pair.url)
                } else {
                    None
                };
                respond_json(
                    request,
                    &PairDto {
                        url: pair.url.clone(),
                        svg,
                        lan: pair.lan,
                    },
                )
            }
            (Method::Get, "/api/browse-keys") => respond_json(request, &browse_keys),
            (Method::Get, "/api/picker-keys") => respond_json(request, &picker_keys),
            (Method::Get, "/api/ask-info") => respond_json(request, &ask_info),
            (Method::Get, "/api/decks") => {
                // Unscoped instances re-resolve the configured decks dir on
                // every fetch, so an edited `decks_dir` takes effect on the
                // next reload/focus without a restart (`{#page-reload-refetches-decks}`).
                let dir = effective_decks_dir(scoped, config_path.as_deref(), &decks_dir);
                if dir != decks_dir {
                    decks_dir = dir;
                }
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
                    respond_json(request, &review_state(reviewing.as_ref(), &store))
                }
            }
            (Method::Post, "/api/select") => {
                match read_selection(&mut request, &decks_dir, &recent) {
                    Some(sel) => {
                        let opts = sel.opts;
                        let paths = vec![sel.deck];
                        // Write to the deck's own store — a workspace's `progress.json`
                        // when they share one, else the global store — the same store
                        // the picker's badges are read from.
                        if let Err(e) = store_for(&paths).map(|s| store = s) {
                            eprintln!("warning: could not open the progress store: {e}");
                            respond_status(request, 400);
                            continue;
                        }
                        match build_walk(&paths) {
                            Ok(Some(wb)) => {
                                let w = Walking::new(wb.walk, wb.grade);
                                let dto = walk_dto(&w);
                                walking = Some(w);
                                reviewing = None;
                                examining = None;
                                respond_json(request, &dto);
                            }
                            Ok(None) => match build(paths, &opts, &store, &mut recent) {
                                Ok(b) => {
                                    // Remember the resolved depth for this deck so a
                                    // plain Learn next time reopens at it (keyed by
                                    // deck subject, like the rest of the deck store).
                                    let resolved = b.session.depth();
                                    let subject = b.decks.keys().next().cloned();
                                    let mut r = Reviewing::new(b);
                                    r.open_augment(store.path());
                                    r.rotate_variant();
                                    if let Some(subject) = subject {
                                        store.set_last_depth(&subject, resolved);
                                        if let Err(e) = store.save() {
                                            eprintln!("warning: could not save progress: {e}");
                                        }
                                    }
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
            // Wipe a row's review progress (the sheet's typed-name gate is
            // client UX; a token holder is trusted — same class as grading).
            (Method::Post, "/api/reset") => {
                #[derive(Deserialize)]
                struct Body {
                    deck: String,
                }
                let Some(body) = serde_json::from_reader::<_, Body>(request.as_reader()).ok()
                else {
                    respond_status(request, 400);
                    continue;
                };
                // Rows resolve to their deck files: a workspace/folder row to
                // its members, a deck row to itself.
                let paths = match resolve_row(&body.deck, &decks_dir, &recent) {
                    Resolved::One(p) => vec![p],
                    Resolved::Many { files, .. } => files,
                    Resolved::Ambiguous | Resolved::Unknown => {
                        respond_status(request, 400);
                        continue;
                    }
                };
                let name = body.deck;
                let decks: Vec<Deck> = match paths.iter().map(Deck::load).collect() {
                    Ok(d) => d,
                    Err(_) => {
                        respond_status(request, 400);
                        continue;
                    }
                };
                let cleared = store_for(&paths)
                    .and_then(|mut s| crate::library::reset_decks(&mut s, decks.iter()));
                match cleared {
                    Ok(n) => {
                        // The in-memory global store may now be stale — reload.
                        if let Ok(s) = store_for(&[]) {
                            store = s;
                        }
                        respond_json(
                            request,
                            &ResetDto {
                                deck: name,
                                cards_cleared: n,
                            },
                        );
                    }
                    Err(_) => respond_status(request, 400),
                }
            }
            // Land an uploaded `.tsv`/`.txt` file via `place_deck`. Strict
            // unlike `generate`'s lenient save: an invalid upload is 400 and
            // no file remains — the upload still exists on the user's
            // device, so nothing is lost by refusing to keep a broken copy.
            (Method::Post, "/api/import") => {
                #[derive(Deserialize)]
                struct Body {
                    name: String,
                    text: String,
                    dest: Option<String>,
                }
                let Some(b) = serde_json::from_reader::<_, Body>(request.as_reader()).ok() else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(dir) = resolve_dest(b.dest.as_deref(), &decks_dir, &recent) else {
                    respond_status(request, 400);
                    continue;
                };
                // `.tsv` converts (Anki export); `.txt` is a deck as-is. Case
                // folded so `FILE.TSV` matches — the browser's file picker
                // accept filter offers upper-case extensions too.
                let lower_name = b.name.to_ascii_lowercase();
                let text = if lower_name.ends_with(".tsv") {
                    match import::tsv_to_deck(&b.text) {
                        Ok(t) => t,
                        Err(_) => {
                            respond_status(request, 400);
                            continue;
                        }
                    }
                } else if lower_name.ends_with(".txt") {
                    b.text
                } else {
                    respond_status(request, 400);
                    continue;
                };
                let place_name = normalize_txt_extension(&b.name, &lower_name);
                match crate::library::place_deck(&dir, &place_name, &text) {
                    Ok(p) if p.parse_error.is_none() => {
                        let deck = p
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        respond_json(
                            request,
                            &ImportDto {
                                deck,
                                cards: p.cards,
                            },
                        );
                    }
                    // Uploads are strict: don't keep an invalid deck around.
                    Ok(p) => {
                        std::fs::remove_file(&p.path).ok();
                        respond_status(request, 400);
                    }
                    Err(_) => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/generate") => {
                #[derive(Deserialize)]
                struct Body {
                    url: String,
                    guidance: Option<String>,
                    dest: Option<String>,
                }
                // A worker may have finished while nobody polled (the page
                // went away) — drain it first, so "finished" means finished
                // even without a GET, and only a live worker 409s.
                if let Some(g) = generating.as_mut() {
                    g.poll();
                }
                if generating.as_ref().is_some_and(|g| g.outcome.is_none()) {
                    respond_status(request, 409); // one costed job at a time
                    continue;
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(b) =
                    body.filter(|b| b.url.starts_with("http://") || b.url.starts_with("https://"))
                else {
                    respond_status(request, 400); // the web generates from URLs only
                    continue;
                };
                let Some(dest) = resolve_dest(b.dest.as_deref(), &decks_dir, &recent) else {
                    respond_status(request, 400);
                    continue;
                };
                // A collision discovered only after the (costed) model call
                // would throw away paid work for nothing — check before
                // spawning, mirroring `library::place_deck`'s stem/extension
                // logic (stage-then-merge: fail fast on what's already
                // knowable, same principle as the CLI's destination guard).
                let name = generate::deck_name(&b.url);
                let stem = name.strip_suffix(".txt").unwrap_or(&name);
                let file = format!("{stem}.txt");
                if dest.join(&file).exists() {
                    respond_json(
                        request,
                        &GenerateDto {
                            phase: "error",
                            deck: None,
                            cards: None,
                            elapsed: Some(0),
                            error: Some(format!(
                                "{file} already exists — rename it or generate into another destination"
                            )),
                        },
                    );
                    continue;
                }
                let mut cfg = generate_cfg.clone();
                if let Some(g) = b
                    .guidance
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                {
                    cfg.extra = Some(g);
                }
                let g = Generating {
                    rx: generate::spawn(b.url.clone(), cfg, ask_cfg.clone()),
                    url: b.url,
                    dest,
                    started: Instant::now(),
                    outcome: None,
                };
                let dto = g.dto();
                generating = Some(g);
                respond_json(request, &dto);
            }
            (Method::Get, "/api/generate") => {
                let Some(g) = generating.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                g.poll();
                respond_json(request, &g.dto());
            }
            (Method::Post, "/api/generate/close") => {
                generating = None; // a running worker finishes and is discarded
                respond_status(request, 200);
            }
            (Method::Post, "/api/share") => {
                #[derive(Deserialize)]
                struct Body {
                    deck: Option<String>,
                }
                // Drain a finished-but-unpolled job first, so a completed send is
                // replaced by the next POST even without an intervening GET —
                // mirroring the `/api/generate` fix (only a *live* job 409s).
                if let Some(s) = sharing.as_mut() {
                    s.poll();
                }
                if sharing.as_ref().is_some_and(|s| s.outcome.is_none()) {
                    respond_status(request, 409); // one share at a time
                    continue;
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let path = match body.and_then(|b| b.deck) {
                    None => Some(decks_dir.clone()),
                    Some(name) => resolved_path(resolve_row(&name, &decks_dir, &recent)),
                };
                let Some(path) = path else {
                    respond_status(request, 400);
                    continue;
                };
                let started = tempfile::tempdir()
                    .map_err(|e| anyhow!("{e}"))
                    .and_then(|tmp| {
                        let to_send = stage_for_share(&path, &tmp)?;
                        let job = share::send_spawn(&to_send)?;
                        Ok(Sharing {
                            job,
                            _stage: tmp,
                            code: None,
                            started: Instant::now(),
                            outcome: None,
                        })
                    });
                match started {
                    Ok(s) => {
                        let dto = s.dto();
                        sharing = Some(s);
                        respond_json(request, &dto);
                    }
                    // Spawn failures (missing binary) surface as an error-phase
                    // job so the sheet shows the install hint, not a bare 400.
                    Err(e) => respond_json(
                        request,
                        &ShareDto {
                            phase: "error",
                            code: None,
                            elapsed: Some(0),
                            error: Some(format!("{e:#}")),
                        },
                    ),
                }
            }
            (Method::Get, "/api/share") => {
                let Some(s) = sharing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                s.poll();
                respond_json(request, &s.dto());
            }
            (Method::Post, "/api/share/close") => {
                // Dropping the (former) job cancels its wormhole child — see
                // `ShareJob`'s `Drop`; `cancel()` here is just for clarity.
                if let Some(s) = sharing.take() {
                    s.job.cancel();
                }
                respond_status(request, 200);
            }
            (Method::Get, "/api/share/zip") => {
                // `request_path` (used for dispatch) already strips the query
                // string, so the plain literal above matches regardless of
                // `?deck=...` — read the param back off the full URL here.
                let name = query_param(request.url(), "deck");
                let path = match &name {
                    None => Some(decks_dir.clone()),
                    Some(n) => resolved_path(resolve_row(n, &decks_dir, &recent)),
                };
                let Some(path) = path else {
                    respond_status(request, 400);
                    continue;
                };
                let zipped = tempfile::tempdir().ok().and_then(|tmp| {
                    let staged = stage_for_share(&path, &tmp).ok()?;
                    let out = tmp.path().join("share.zip");
                    share::zip_to(&staged, &out).ok()?;
                    std::fs::read(&out).ok()
                });
                match zipped {
                    Some(bytes) => {
                        let stem = name
                            .as_deref()
                            .map(|n| n.rsplit('/').next().unwrap_or(n))
                            .unwrap_or("shared-decks");
                        respond_download(request, bytes, "application/zip", &format!("{stem}.zip"));
                    }
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/receive") => {
                #[derive(Deserialize)]
                struct Body {
                    code: String,
                    dest: Option<String>,
                }
                // Drain a finished-but-unpolled job first — same fix as
                // generate/share: only a *live* job 409s.
                if let Some(r) = receiving.as_mut() {
                    r.poll();
                }
                if receiving.as_ref().is_some_and(|r| r.outcome.is_none()) {
                    respond_status(request, 409); // one receive at a time
                    continue;
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let Some(b) = body else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(dest) = resolve_dest(b.dest.as_deref(), &decks_dir, &recent) else {
                    respond_status(request, 400);
                    continue;
                };
                let started = tempfile::tempdir()
                    .map_err(|e| anyhow!("{e}"))
                    .and_then(|tmp| {
                        let job = share::receive_spawn(&b.code, tmp.path())?;
                        Ok(Receiving {
                            job,
                            tmp,
                            dest,
                            started: Instant::now(),
                            outcome: None,
                        })
                    });
                match started {
                    Ok(r) => {
                        let dto = r.dto();
                        receiving = Some(r);
                        respond_json(request, &dto);
                    }
                    // Spawn failures (missing binary) surface as an error-phase
                    // job so the sheet shows the install hint, not a bare 400.
                    Err(e) => respond_json(
                        request,
                        &ReceiveDto {
                            phase: "error",
                            landed: None,
                            stripped: Vec::new(),
                            elapsed: Some(0),
                            error: Some(format!("{e:#}")),
                        },
                    ),
                }
            }
            (Method::Get, "/api/receive") => {
                let Some(r) = receiving.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.poll();
                respond_json(request, &r.dto());
            }
            (Method::Post, "/api/receive/close") => {
                // Dropping the (former) job cancels its wormhole child — see
                // `ShareJob`'s `Drop`; `cancel()` here is just for clarity.
                if let Some(r) = receiving.take() {
                    r.job.cancel();
                }
                respond_status(request, 200);
            }
            (Method::Post, "/api/receive/zip") => {
                // `request_path` (used for dispatch) already strips the query
                // string (Task 10 confirmed), so the plain literal above
                // matches regardless of `?dest=...` — read the param back off
                // the full URL here, same as `/api/share/zip`.
                const MAX_ZIP: usize = 50 * 1024 * 1024;
                if request.body_length().is_some_and(|l| l > MAX_ZIP) {
                    respond_status(request, 400);
                    continue;
                }
                let Some(dest) = resolve_dest(
                    query_param(request.url(), "dest").as_deref(),
                    &decks_dir,
                    &recent,
                ) else {
                    respond_status(request, 400);
                    continue;
                };
                // `body_length` can lie or be absent, so `read_capped` also
                // bounds the actual read, not just the declared length.
                let Some(bytes) = read_capped(request.as_reader(), MAX_ZIP) else {
                    respond_status(request, 400);
                    continue;
                };
                // `land_received`'s collision check is check-then-act; safe
                // only because this server loop is single-threaded — do not
                // introduce threads here (see `Receiving::poll`'s note).
                let landed = tempfile::tempdir().ok().and_then(|tmp| {
                    let zip_path = tmp.path().join("got.zip");
                    std::fs::write(&zip_path, &bytes).ok()?;
                    let scratch = tmp.path().join("out");
                    std::fs::create_dir_all(&scratch).ok()?;
                    share::unzip_to(&zip_path, &scratch).ok()?;
                    share::land_received(&scratch, &dest).ok()
                });
                match landed {
                    Some((landed, stripped)) => respond_json(
                        request,
                        &ReceiveDto {
                            phase: "done",
                            landed: Some(landed),
                            stripped,
                            elapsed: Some(0),
                            error: None,
                        },
                    ),
                    None => respond_status(request, 400),
                }
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
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            (Method::Post, "/api/grade") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                match read_grade(&mut request) {
                    Some(grade) => {
                        let now = now_ms();
                        r.session.grade(&mut store, grade, now);
                        // Refresh the deck's per-depth badge earn dates from this
                        // session's cards (high-water first-earn marks; badges gate
                        // nothing). Keyed by deck subject, like the rest of the
                        // deck-level store (exam mastery, last depth).
                        if let Some(subject) = r.files.paths.keys().next() {
                            store::note_badges(&mut store, subject, r.session.cards(), now);
                        }
                        if let Err(e) = store.save() {
                            eprintln!("warning: could not save progress: {e}");
                        }
                        r.rotate_variant(); // a fresh phrasing for the next card
                        respond_json(request, &review_state(reviewing.as_ref(), &store));
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
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            (Method::Post, "/api/acquire") => {
                // Acknowledge a never-seen card: record it as acquired (no grade)
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
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            (Method::Post, "/api/check") => {
                let Some(r) = reviewing.as_ref() else {
                    respond_status(request, 409);
                    continue;
                };
                // Grade the typed lines against the current card: normalized then
                // compared exactly, no edit-distance tolerance. Pure evidence —
                // like choose, this only checks; the learner-final grade is applied
                // separately on Continue via `/api/grade`. `ordered` (TypeLine, the
                // `% reveal: line` reconstruct path) pairs line-by-position; the
                // default matches each input to its closest expected line so a
                // multi-item answer can be entered in any order.
                #[derive(Deserialize)]
                struct Body {
                    lines: Vec<String>,
                    #[serde(default)]
                    ordered: bool,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                let result = body.and_then(|body| {
                    let card = r.session.current()?;
                    let results: Vec<TypedResult> = if body.ordered {
                        grade_lines_ordered(&body.lines, &card.back)
                    } else {
                        grade_lines_unordered(&body.lines, &card.back)
                    };
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
                // Just reports which option is correct (the question is rebuilt from
                // the card id via `current_question`, so it matches the one served
                // by `review_state` for every question shape). The grade is applied
                // later via /api/grade on Continue, so the session stays on this card
                // during the result — Remove still works on it.
                let picked = read_index(&mut request).and_then(|chosen| {
                    let card = r.session.current()?.clone();
                    let correct = current_question(r, &store, &card)?.correct;
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
                respond_json(request, &review_state(reviewing.as_ref(), &store));
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
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            (Method::Post, "/api/restart") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.session.restart(&store, now_ms());
                r.rotate_variant(); // a fresh phrasing for the new session's first card
                respond_json(request, &review_state(reviewing.as_ref(), &store));
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
                                        // One shape per endpoint: the cooldown is an
                                        // ExamDto in its own phase, not an untagged
                                        // {cooldown_ms} the client must key-sniff.
                                        respond_json(request, &cooldown_dto(&deck.subject, ms));
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
                            // Fact-deck pre-flight: confirm the configured
                            // backend can reach every `% source:` before
                            // starting the sitting, so a capability gap is a
                            // clean refusal at launch, not an error surfaced
                            // mid-exam through the background job's poll.
                            if exam::ensure_backend_can_examine(&deck, &ask_cfg).is_err() {
                                respond_status(request, 409);
                                continue;
                            }
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
                let parent = ex.deck_path.parent().unwrap_or_else(|| Path::new(""));
                let retire_after_days = review_cfg.for_workspace(parent).retire_after_days;
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
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            // ── Deck augmentation (the picker's "Augment" action, decks only) ──
            // Open a deck's Augment screen and report what its cache holds. Resolves
            // the deck through the catalog (incl. workspace members) like the exam.
            (Method::Post, "/api/augment/open") => {
                #[derive(Deserialize)]
                struct Body {
                    deck: String,
                }
                let Some(body) = serde_json::from_reader::<_, Body>(request.as_reader()).ok()
                else {
                    respond_status(request, 400);
                    continue;
                };
                let Some(path) = resolved_path(resolve_row(&body.deck, &decks_dir, &recent)) else {
                    respond_status(request, 400);
                    continue;
                };
                let name = body.deck;
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
                respond_json(request, &review_state(reviewing.as_ref(), &store));
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
                // Every closer returns the picker StateDto — one teardown rule.
                respond_json(request, &review_state(reviewing.as_ref(), &store));
            }
            _ => respond_status(request, 404),
        }
    }
    Ok(())
}

// ── Trace walks (in-page, from the picker) ──────────────────────────────────
//
// A single walk of one trace deck: predict → reveal a live excerpt → grade →
// compress. There is no deck-selection screen (one deck, one walk). The
// frontend-agnostic `Walk` state machine carries the logic; this is a thin web
// reader over it. Live Claude grading (`--grade`) is the only async step, so it
// runs on a background thread and the page polls `GET /api/walk` while
// `thinking`, like the exam.

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
        verdict: w.grade_result.as_ref().map(|(d, _)| delta_name(*d)),
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

/// The multiple-choice question for `card` right now, or `None` when it doesn't
/// render as a pick (or has too few distractors — the client then falls back to
/// attempt→reveal). The one place the question is built, so `review_state`'s
/// served options and `/api/choose`'s graded correct index stay in lockstep; the
/// `card.id()` seed keeps it stable across both requests without server caching.
///
/// - **Acquire** (first encounter): a recognition MC only under the strict bar (atomic answer + a
///   full set of cached AI distractors), and — spec §4.6 — never for a card already recognized
///   (such a card keeps its store entry, so it isn't acquired cold anyway).
/// - **Recognize session** (non-acquire): the shape follows the card's `% reveal:` — `Line` picks
///   the next line among the card's own lines; `Flip` and `Cloze` pick the back among sibling backs
///   via plain `build` (an expanded cloze sub-card's back IS its gap text — T6 review).
/// - Any other depth renders no pick.
fn current_question(r: &Reviewing, store: &Store, card: &Card) -> Option<ChoiceQuestion> {
    let ai = r.augment.distractors(card.id());
    if store.get(card.id()).is_none() {
        // Acquire on-ramp. A recognized card has a store entry, so it never lands
        // here — this branch already established `store.get(card.id())` is `None`.
        return choice::recognition_question(card, r.session.cards(), card.id(), ai);
    }
    if r.session.depth() != Depth::Recognize {
        return None;
    }
    match card.reveal.unwrap_or_default() {
        // Progressive next-line tracking is the client's (Task 9); the state offers
        // the first line, and `/api/choose` rebuilds the same question.
        Reveal::Line => choice::line_question(card, 0, card.id(), ai),
        Reveal::Flip | Reveal::Cloze => choice::build(card, r.session.cards(), card.id(), ai),
    }
}

/// Builds the state payload. In the select phase (`reviewing` is `None`) it
/// reports `phase: "select"` with no card; otherwise it serializes the live
/// session and store. For a choice card it also builds the options via
/// [`current_question`], seeded by the card id so they are stable across the
/// `/api/state` and `/api/choose` requests without any server-side caching.
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
            depth: depth_name(Depth::default()),
            input: input_name(Input::default()),
            remaining: 0,
            initial: 0,
            reviews: 0,
            passed: 0,
            failed: 0,
            exam_due: Vec::new(),
            can_restart: false,
            promotable: false,
            label: "select decks".to_string(),
        };
    };
    let session = &r.session;
    let card = session.current();
    // The session owns its depth (Recognize | Recall | Reconstruct); the concrete
    // check derives from that and the card's authored `% reveal:` (spec §4).
    let depth = session.depth();
    let mode = card
        .map(|c| depth::check_for(c.reveal.unwrap_or_default(), depth, c))
        .unwrap_or_default();
    // A never-seen card is *acquired* (an attempt, then reveal), not quizzed cold.
    let acquire = session.current_unseen(store);
    // The multiple-choice options, if this card renders as a pick — the acquire
    // on-ramp or a Recognize session. `current_question` is the single source both
    // this state and `/api/choose` build from (same `c.id()` seed), so the served
    // options and the graded correct index can never diverge.
    let choices = card
        .and_then(|c| current_question(r, store, c))
        .map(|q| q.options);
    // Explain mode reveals the key points as a tick-each-line checklist whose
    // coverage derives the grade: the cached `keypoints` augment when present,
    // else the card's own back lines (that IS the multi-line reconstruct check —
    // spec §4.3). Any other mode keeps the plain reveal; never on a first
    // encounter, where acquiring just reveals the answer.
    let keypoints = if !acquire && mode == Mode::Explain {
        card.map(|c| {
            r.augment
                .keypoints(c.id())
                .map(<[String]>::to_vec)
                .unwrap_or_else(|| c.back.clone())
        })
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
        depth: depth_name(depth),
        input: input_name(card.and_then(|c| c.input).unwrap_or_default()),
        remaining: session.remaining(),
        initial: session.initial_size,
        reviews: session.stats.reviews,
        passed: session.stats.passed,
        failed: session.stats.failed,
        exam_due,
        can_restart: session.has_due_now(store, now_ms()),
        promotable: session.current_is_virtual(store),
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

/// A single loose deck as a selection row, its badge/lock/gating from the
/// shared [`picker::deck_status`].
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
            let last_depth = depth_name(store.last_depth(&deck.subject).unwrap_or_default());
            DeckItemDto {
                name: e.name.clone(),
                label: e.label.clone(),
                meta: Some(s.badge),
                state: state_name(s.state),
                locked: s.locked,
                reviewable: s.reviewable,
                reviewable_recognize: s.reviewable_recognize,
                reviewable_recall: s.reviewable_recall,
                reviewable_reconstruct: s.reviewable_reconstruct,
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
                badge_depth: s.badge_depth.map(depth_name),
                badge_dotted: s.badge_dotted,
                new_cards: s.new_cards,
                last_depth,
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
            reviewable_recognize: true,
            reviewable_recall: true,
            reviewable_reconstruct: true,
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
            badge_depth: None,
            badge_dotted: false,
            new_cards: false,
            last_depth: depth_name(Depth::default()),
        },
    }
}

/// A workspace/folder's members as an unlock dependency tree (the drill-in
/// list): each member nests under the `% requires:` that gates it, siblings
/// startable-first, carrying an `indent` for the tree nesting. Badges/locks come from
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
    // Load each member deck once, deriving its status, whether it has a
    // topology, and its last-used session depth from the same parse.
    let loaded: Vec<(Option<picker::DeckStatus>, bool, &'static str)> = paths
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
            // Subject-keyed like `deck_item_dto`, from the workspace's own store.
            let last_depth = match (store.as_ref(), deck.as_ref()) {
                (Some(st), Some(d)) => st.last_depth(&d.subject).unwrap_or_default(),
                _ => Depth::default(),
            };
            (status, has_topology, depth_name(last_depth))
        })
        .collect();
    // Order siblings startable-first (blocked = locked, or — when gating —
    // nothing to review), then by label.
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
            let indent = prefix.chars().count() / 3;
            let has_topology = loaded[i].1;
            let last_depth = loaded[i].2;
            match &loaded[i].0 {
                Some(s) => MemberDto {
                    name: m.name.clone(),
                    label: m.label.clone(),
                    meta: Some(s.badge.clone()),
                    state: state_name(s.state),
                    locked: s.locked,
                    reviewable: s.reviewable,
                    reviewable_recognize: s.reviewable_recognize,
                    reviewable_recall: s.reviewable_recall,
                    reviewable_reconstruct: s.reviewable_reconstruct,
                    mastered: s.mastered,
                    is_trace: s.is_trace,
                    examable: s.examable,
                    has_exam: s.has_exam,
                    indent,
                    tree: prefix.clone(),
                    has_topology,
                    badge_depth: s.badge_depth.map(depth_name),
                    badge_dotted: s.badge_dotted,
                    new_cards: s.new_cards,
                    last_depth,
                },
                // A member that failed to load: the same neutral defaults as
                // `deck_item_dto`'s failed-load fallback.
                None => MemberDto {
                    name: m.name.clone(),
                    label: m.label.clone(),
                    meta: None,
                    state: "new",
                    locked: false,
                    reviewable: true,
                    reviewable_recognize: true,
                    reviewable_recall: true,
                    reviewable_reconstruct: true,
                    mastered: false,
                    is_trace: false,
                    examable: false,
                    has_exam: false,
                    indent,
                    tree: prefix.clone(),
                    has_topology,
                    badge_depth: None,
                    badge_dotted: false,
                    new_cards: false,
                    last_depth,
                },
            }
        })
        .collect()
}

/// Builds the deck-selection catalog's three sections — workspaces (each with
/// its last-progress time), recent loose decks, and plain folders — each
/// deck's badge/lock from `store`. `with_lock` is false for the browse
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

/// The decks dir this catalog fetch should serve. A scoped instance
/// (`alix <dir>`) is pinned to its root forever; a config-derived instance
/// follows a live `decks_dir` edit on the next reload (the ⟳ button and the
/// focus-refresh both re-fetch /api/decks). A config that no longer parses
/// keeps the current dir — the picker must never go down over a typo; the
/// doctor sheet is where the parse error surfaces.
fn effective_decks_dir(scoped: bool, config_path: Option<&Path>, current: &Path) -> PathBuf {
    if scoped {
        return current.to_path_buf();
    }
    Config::load(config_path)
        .ok()
        .and_then(|c| c.decks_dir())
        .unwrap_or_else(|| current.to_path_buf())
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
                reviewable_recognize: true,
                reviewable_recall: true,
                reviewable_reconstruct: true,
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
                badge_depth: None,
                badge_dotted: false,
                new_cards: false,
                last_depth: depth_name(Depth::default()),
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
/// topology and/or region, and at a chosen session `depth` (absent = the deck's
/// last-used depth, defaulting to Recall).
struct Selection {
    deck: PathBuf,
    opts: SelectOptions,
}

/// The per-launch choices a selection carries beyond which deck: the picker's
/// depth pick, focus-drawer topology/region scope, the cram tick-box, and
/// optional pacing overrides (absent → the instance's CLI/config values).
#[derive(Default)]
pub struct SelectOptions {
    pub topology: Option<String>,
    pub region: Option<String>,
    pub depth: Option<Depth>,
    pub cram: bool,
    pub max_new: Option<usize>,
    pub limit: Option<usize>,
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
        #[serde(default)]
        depth: Option<Depth>,
        #[serde(default)]
        cram: bool,
        #[serde(default)]
        max_new: Option<usize>,
        #[serde(default)]
        limit: Option<usize>,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    if body.deck.is_empty() {
        return None;
    }
    // Covers top-level decks/workspaces and every workspace's members (by
    // their qualified `<workspace>/<file>` key), so a member selection from
    // inside a workspace resolves safely too. Unknown, crafted, and ambiguous
    // (duplicated bare) names all resolve to nothing here — `/api/select`
    // and `/api/browse` already answer 400 on `None`; `/api/deck-topology`
    // falls back to an empty DTO.
    let deck = resolved_path(resolve_row(&body.deck, decks_dir, recent))?;
    Some(Selection {
        deck,
        opts: SelectOptions {
            topology: body.topology,
            region: body.region,
            depth: body.depth,
            cram: body.cram,
            max_new: body.max_new,
            limit: body.limit,
        },
    })
}

/// One resolution map for every name-taking endpoint. Bare names that occur
/// more than once (two containers holding decks with the same file name)
/// resolve to `Ambiguous` — callers answer 400 and the client uses the
/// qualified `<workspace>/<file>` key instead; silently picking one of two
/// same-named decks was wrong everywhere and dangerous behind `/api/reset`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resolved {
    One(PathBuf),
    /// A container row (workspace/folder): its own directory and its member
    /// deck files — so no caller ever has to reconstruct one from the other.
    Many {
        dir: PathBuf,
        files: Vec<PathBuf>,
    },
    Ambiguous,
    Unknown,
}

/// Resolves a requested name against the live catalog — the one lookup every
/// name-taking endpoint shares. Qualified member keys (`<workspace>/<file>`)
/// and bare top-level row keys never collide (a filename can't contain `/`),
/// so a bare-name collision can only happen among top-level rows; a member's
/// qualified key always resolves regardless. A bare row name seen more than
/// once flips that key to `Ambiguous` for the rest of this call — no name
/// ever silently picks one of several same-named rows.
fn resolve_row(name: &str, decks_dir: &Path, recent: &RecentDecks) -> Resolved {
    let mut known: HashMap<String, Resolved> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    for e in picker::catalog(decks_dir, recent) {
        for m in &e.members {
            known.insert(m.name.clone(), Resolved::One(m.path.clone()));
        }
        let row = if e.members.is_empty() {
            Resolved::One(e.path)
        } else {
            Resolved::Many {
                dir: e.path.clone(),
                files: e.members.iter().map(|m| m.path.clone()).collect(),
            }
        };
        if seen.insert(e.name.clone()) {
            known.insert(e.name, row);
        } else {
            known.insert(e.name, Resolved::Ambiguous);
        }
    }
    known.get(name).cloned().unwrap_or(Resolved::Unknown)
}

/// Collapses a [`Resolved`] to the single path `read_selection`/augment/share/
/// share-zip need: a plain deck's own file, or — for a workspace/folder row —
/// its directory, matching what these call sites did before `resolve_row`
/// existed (they used the row's own path rather than expanding to members;
/// `/api/reset` is the one caller that wants the member list, so it matches on
/// `Resolved` directly instead of going through this).
fn resolved_path(resolved: Resolved) -> Option<PathBuf> {
    match resolved {
        Resolved::One(p) => Some(p),
        Resolved::Many { dir, .. } => Some(dir),
        Resolved::Ambiguous | Resolved::Unknown => None,
    }
}

/// Resolves an add-sheet destination: absent/empty → the served root; a name
/// → a workspace/folder row's directory, looked up through the same catalog
/// `/api/select` uses (never a client-crafted path). `None` = unknown name, or
/// a name duplicated across containers (same rejection `resolve_row` applies
/// to bare names — dest names are top-level-only, so `catalog` can surface the
/// same duplication) — the caller rejects with 400. Tasks 9 and 11 (`generate`,
/// `receive`) reuse this.
fn resolve_dest(dest: Option<&str>, decks_dir: &Path, recent: &RecentDecks) -> Option<PathBuf> {
    let Some(name) = dest.filter(|d| !d.is_empty()) else {
        return Some(decks_dir.to_path_buf());
    };
    let mut matches = picker::catalog(decks_dir, recent)
        .into_iter()
        .filter(|e| e.name == name && e.path.is_dir());
    let first = matches.next()?;
    if matches.next().is_some() {
        return None; // ambiguous: more than one dir row shares this name
    }
    Some(first.path)
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
    let parent = deck.path.parent().unwrap_or_else(|| Path::new(""));
    let review = review.for_workspace(parent);
    let by_id: HashMap<u64, &Card> = deck.cards.iter().map(|c| (c.id(), c)).collect();
    let deck_ids: HashSet<u64> = by_id.keys().copied().collect();
    let scheduler = Fsrs::new(review.retention);
    let now = now_ms();
    // Cards in a region resolved back to the deck (ids absent from the deck —
    // e.g. a topology built before an edit — are skipped).
    // Pinned to Recall, like `card_strengths`/`retrievability` above — the
    // focus drawer is a deck-wide signal (spec §4.5), not a per-session one.
    let due_of = |ids: &[u64]| {
        let cards: Vec<&Card> = ids.iter().filter_map(|id| by_id.get(id).copied()).collect();
        crate::session::count_reviewable(
            &cards,
            store,
            &scheduler,
            Depth::Recall,
            now,
            review.retire_after_days,
        )
    };
    // Whole-deck due count: the deck's own cards, plus any of its virtual
    // (remediation) cards that are due — never affecting deck size/composition.
    let deck_due = deck
        .cards
        .iter()
        .filter(|c| {
            crate::session::is_reviewable(
                c,
                store,
                &scheduler,
                Depth::Recall,
                now,
                review.retire_after_days,
            )
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

/// Reads `reader` to end, capped at `cap` bytes: `None` if the read errors or
/// the body exceeds the cap. `take(cap + 1)` lets an oversized body read one
/// byte past the cap, which the length check below catches — so a reader
/// whose declared length lies (or has none) is still bounded by the actual
/// bytes read, not by what it claims.
fn read_capped(reader: impl Read, cap: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    if reader.take(cap as u64 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > cap {
        None
    } else {
        Some(bytes)
    }
}

/// The app shell and its assets must never be served stale: alix ships no
/// version in its URLs, so after an upgrade a heuristically-cached page keeps
/// showing the OLD web app (seen in the wild: a week-old review.html surviving
/// a `make install`). `no-cache` forces revalidation on every load — cheap on
/// localhost — and `no-store` keeps live JSON state out of the cache entirely.
fn cache_header(policy: &'static [u8]) -> Header {
    Header::from_bytes(&b"Cache-Control"[..], policy).unwrap()
}

fn respond_json<T: Serialize>(request: Request, value: &T) {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    let header = Header::from_bytes(
        &b"Content-Type"[..],
        &b"application/json; charset=utf-8"[..],
    )
    .unwrap();
    let _ = request.respond(
        Response::from_string(body)
            .with_header(header)
            .with_header(cache_header(b"no-store")),
    );
}

fn respond_html(request: Request, html: &str) {
    let header =
        Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
    let _ = request.respond(
        Response::from_string(html.to_string())
            .with_header(header)
            .with_header(cache_header(b"no-cache")),
    );
}

/// Serves a static text asset (the shared `theme.css` / `theme.js`) with the
/// given content type.
fn respond_asset(request: Request, body: &str, content_type: &str) {
    let header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap();
    let _ = request.respond(
        Response::from_string(body.to_string())
            .with_header(header)
            .with_header(cache_header(b"no-cache")),
    );
}

fn respond_status(request: Request, code: u16) {
    let _ = request.respond(Response::from_string(String::new()).with_status_code(code));
}

fn respond_bytes(request: Request, bytes: Vec<u8>, content_type: &str) {
    let header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap();
    let _ = request.respond(Response::from_data(bytes).with_header(header));
}

/// A Content-Disposition-safe file name: ASCII only (tiny_http header
/// values must be), quotes/backslashes/control characters dropped, and
/// never empty — a fully non-ASCII name falls back to a generic one.
///
/// The alphanumeric check looks only at the stem (before the last `.`):
/// an extension alone (e.g. a non-ASCII name filtered down to `.zip`) is
/// not a real file name, so it also falls back.
fn download_filename(name: &str) -> String {
    let safe: String = name
        .chars()
        .filter(|c| c.is_ascii() && !c.is_ascii_control() && *c != '"' && *c != '\\')
        .collect();
    let stem = match safe.rfind('.') {
        Some(idx) => &safe[..idx],
        None => safe.as_str(),
    };
    if !stem.chars().any(|c| c.is_ascii_alphanumeric()) {
        "decks.zip".to_string()
    } else {
        safe
    }
}

/// Normalizes an uploaded name's `.txt` extension to lower case before it
/// reaches [`crate::library::place_deck`], whose suffix-strip is
/// case-sensitive (a locked contract) — without this, `FILE.TXT` would save
/// as `FILE.TXT.txt`. `lower_name` is the already-lowercased name, used only
/// to test the ending; slicing 4 bytes off `name` is safe because a matched
/// `.txt` ending means the last 4 bytes are that same ASCII extension
/// (lowercasing never changes a string's byte length).
fn normalize_txt_extension(name: &str, lower_name: &str) -> String {
    if lower_name.ends_with(".txt") {
        format!("{}.txt", &name[..name.len() - 4])
    } else {
        name.to_string()
    }
}

/// Like [`respond_bytes`], but marks the response as a file to save rather
/// than render inline — the zip export is the one non-JSON API response, and
/// a browser only offers "save as" with `Content-Disposition: attachment`.
fn respond_download(request: Request, bytes: Vec<u8>, content_type: &str, filename: &str) {
    let content_type_header =
        Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap();
    let disposition = format!("attachment; filename=\"{}\"", download_filename(filename));
    let response = Response::from_data(bytes).with_header(content_type_header);
    let _ = match Header::from_bytes(&b"Content-Disposition"[..], disposition.as_bytes()) {
        Ok(disposition_header) => request.respond(response.with_header(disposition_header)),
        Err(_) => request.respond(response),
    };
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
    fn a_non_ascii_deck_name_yields_an_ascii_download_filename() {
        let name = download_filename("mövenpick-decks.zip");
        assert!(name.is_ascii());
        assert!(name.ends_with(".zip"));
    }

    #[test]
    fn a_fully_non_ascii_name_falls_back_to_a_generic_filename() {
        assert_eq!(download_filename("日本語.zip"), "decks.zip");
    }

    #[test]
    fn quotes_and_backslashes_are_stripped_from_download_filenames() {
        let name = download_filename("weird\"na\\me.zip");
        assert!(!name.contains('"'));
        assert!(!name.contains('\\'));
    }

    #[test]
    fn an_uppercase_txt_extension_is_lowered_before_placing() {
        let name = "FILE.TXT";
        let lower = name.to_ascii_lowercase();
        assert_eq!(normalize_txt_extension(name, &lower), "FILE.txt");
    }

    #[test]
    fn a_lowercase_txt_extension_passes_through_unchanged() {
        let name = "deck.txt";
        let lower = name.to_ascii_lowercase();
        assert_eq!(normalize_txt_extension(name, &lower), "deck.txt");
    }

    #[test]
    fn a_tsv_name_is_left_untouched_by_the_txt_normalizer() {
        let name = "EXPORT.TSV";
        let lower = name.to_ascii_lowercase();
        assert_eq!(normalize_txt_extension(name, &lower), "EXPORT.TSV");
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
    fn resolve_row_resolves_a_unique_bare_deck_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("solo.txt"), "# f\n\tb\n").unwrap();
        let recent = RecentDecks::load(dir.path().join("recent.json"));

        assert_eq!(
            Resolved::One(dir.path().join("solo.txt")),
            resolve_row("solo.txt", dir.path(), &recent)
        );
    }

    #[test]
    fn resolve_row_resolves_an_unknown_name_to_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let recent = RecentDecks::load(dir.path().join("recent.json"));

        assert_eq!(
            Resolved::Unknown,
            resolve_row("../etc/passwd", dir.path(), &recent)
        );
    }

    #[test]
    fn resolve_row_resolves_a_workspace_row_to_many_with_every_member_file() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("english");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("a.txt"), "# a\n\tb\n").unwrap();
        std::fs::write(ws.join("b.txt"), "# c\n\td\n").unwrap();
        std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"English\"\n").unwrap();
        let recent = RecentDecks::load(dir.path().join("recent.json"));

        assert_eq!(
            Resolved::Many {
                dir: ws.clone(),
                files: vec![ws.join("a.txt"), ws.join("b.txt")],
            },
            resolve_row("english", dir.path(), &recent)
        );
    }

    #[test]
    fn resolve_row_resolves_a_manifest_only_dir_with_no_members_to_unknown() {
        // A folder with an `alix.toml` manifest but zero `*.txt` decks:
        // `workspace::has_decks` requires at least one member, so
        // `picker::catalog` never surfaces this row at all — it can't reach
        // the old `vec![e.path]`/`One` fallback because it never becomes a
        // catalog entry in the first place.
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("empty-ws");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"Empty\"\n").unwrap();
        let recent = RecentDecks::load(dir.path().join("recent.json"));

        assert!(picker::catalog(dir.path(), &recent).is_empty());
        assert_eq!(
            Resolved::Unknown,
            resolve_row("empty-ws", dir.path(), &recent)
        );
    }

    #[test]
    fn resolve_row_rejects_a_bare_name_duplicated_across_two_containers() {
        // Two real `a.txt` decks that share a bare name but live in different
        // containers: one under `decks_dir`, the other reached only via
        // `recent` (so the catalog surfaces both under the same key "a.txt").
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "# f\n\tb\n").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(elsewhere.path().join("a.txt"), "# g\n\th\n").unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        recent.record(&[elsewhere.path().join("a.txt")], 1000);

        assert_eq!(
            Resolved::Ambiguous,
            resolve_row("a.txt", dir.path(), &recent)
        );
    }

    #[test]
    fn resolve_row_resolves_a_qualified_member_name_even_when_its_bare_workspace_name_is_duplicated()
     {
        // "english" collides across two containers (ambiguous bare name), but
        // the qualified member key "english/a.txt" is unaffected — qualified
        // and bare names are disjoint namespaces (a filename can't contain
        // `/`), so the collision on one never bleeds into the other.
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("english");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("a.txt"), "# a\n\tb\n").unwrap();
        std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"English\"\n").unwrap();

        let other_ws = tempfile::tempdir().unwrap();
        let other_english = other_ws.path().join("english");
        std::fs::create_dir(&other_english).unwrap();
        std::fs::write(other_english.join("z.txt"), "# z\n\ty\n").unwrap();
        std::fs::write(
            other_english.join(crate::workspace::MANIFEST),
            "title = \"Other English\"\n",
        )
        .unwrap();

        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        recent.record(&[other_english], 1000);

        assert_eq!(
            Resolved::Ambiguous,
            resolve_row("english", dir.path(), &recent)
        );
        assert_eq!(
            Resolved::One(ws.join("a.txt")),
            resolve_row("english/a.txt", dir.path(), &recent)
        );
    }

    // Note: the workspace-row-to-`Many{dir,files}` case the roadmap's
    // reset-specific test asked for (every member file, `dir` == row path) is
    // already asserted in full by
    // `resolve_row_resolves_a_workspace_row_to_many_with_every_member_file`
    // above (extended to cover `dir` in the 0b1b859 review follow-up) —
    // no new assertion here would add coverage, so none is added.

    #[test]
    fn a_drained_job_ignores_further_messages_without_replacing() {
        // Tests the guard at the top of `poll()`: if `self.outcome.is_some()`,
        // return immediately without draining — the drain-once law. All three
        // job POSTs rely on this: a repeat poll must never re-place the deck or
        // re-run the outcome, else a second message would clobber the first.
        // Mutation test: without the guard, poll #2 would drain the second
        // message, call place_deck again (placing a second file), and asserts 2
        // and 3 would fail.
        let dest = tempfile::tempdir().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();

        // First message: place a deck.
        tx.send(Ok("# f\n\tb\n".to_string())).unwrap();
        let mut g = Generating {
            rx,
            url: "https://example.com/some-article".to_string(),
            dest: dest.path().to_path_buf(),
            started: Instant::now(),
            outcome: None,
        };

        // Poll #1: outcome set, one deck placed.
        g.poll();
        assert!(g.outcome.is_some());
        let files_after_poll_1: Vec<_> = std::fs::read_dir(dest.path()).unwrap().collect();
        assert_eq!(1, files_after_poll_1.len());

        // Send a second, distinguishable message (would place a different deck
        // if poll #2 tried to drain it).
        tx.send(Ok("# other\n\tanswer\n".to_string())).unwrap();

        // Poll #2: guard should short-circuit, leaving the second message queued.
        let first_outcome = g.outcome.clone();
        g.poll();
        assert_eq!(first_outcome, g.outcome, "outcome must stay unchanged");
        let files_after_poll_2: Vec<_> = std::fs::read_dir(dest.path()).unwrap().collect();
        assert_eq!(1, files_after_poll_2.len(), "still only one placed file");

        // The second message is still queued (guard never called try_recv).
        assert!(
            g.rx.try_recv().is_ok(),
            "guard short-circuited before draining the second message"
        );
    }

    #[test]
    fn the_zip_upload_cap_accepts_the_boundary_and_rejects_one_past_it() {
        const CAP: usize = 8;
        let at_cap = read_capped(&[7u8; CAP][..], CAP);
        assert_eq!(Some(CAP), at_cap.map(|b| b.len()));

        assert!(read_capped(&[7u8; CAP + 1][..], CAP).is_none());

        // No fixed length at all (a body whose declared length lies, or is
        // absent) is still bounded by the `take()` ceiling: `read_capped`
        // never reads more than `cap + 1` bytes before rejecting, so an
        // endless reader is caught rather than read to exhaustion.
        assert!(read_capped(std::io::repeat(7), CAP).is_none());
    }

    #[test]
    fn resolve_dest_falls_back_to_decks_dir_and_rejects_unknown_names() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("english");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("a.txt"), "# a\n\tb\n").unwrap();
        let recent = RecentDecks::load(dir.path().join("recent.json"));

        // Absent/empty → the served root, without touching the catalog.
        assert_eq!(
            resolve_dest(None, dir.path(), &recent),
            Some(dir.path().to_path_buf())
        );
        assert_eq!(
            resolve_dest(Some(""), dir.path(), &recent),
            Some(dir.path().to_path_buf())
        );
        // A known workspace name → its directory.
        assert_eq!(
            resolve_dest(Some("english"), dir.path(), &recent),
            Some(ws.clone())
        );
        // An unknown name (or a crafted path) resolves to nothing.
        assert_eq!(
            resolve_dest(Some("no-such-workspace"), dir.path(), &recent),
            None
        );
        assert_eq!(resolve_dest(Some("../etc"), dir.path(), &recent), None);
    }

    #[test]
    fn resolve_dest_rejects_a_dir_name_duplicated_across_two_containers() {
        // Same class of collision `resolve_row` rejects for bare deck names:
        // `resolve_dest` also scans top-level catalog rows, so a workspace
        // reached via `recent` can share a name with one physically inside
        // `decks_dir` — silently picking either would be the same class of
        // bug this task closes for names.
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("english");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join("a.txt"), "# a\n\tb\n").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let other_english = elsewhere.path().join("english");
        std::fs::create_dir(&other_english).unwrap();
        std::fs::write(other_english.join("z.txt"), "# z\n\ty\n").unwrap();
        let mut recent = RecentDecks::load(dir.path().join("recent.json"));
        recent.record(&[other_english], 1000);

        assert_eq!(resolve_dest(Some("english"), dir.path(), &recent), None);
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

    /// Builds a `Reviewing` over a parsed deck at a chosen depth, sharing `store`
    /// (seed it before calling so the session sees the seeded state).
    fn reviewing_at(deck: PathBuf, cards: Vec<Card>, store: &Store, depth: Depth) -> Reviewing {
        let session = Session::new(
            cards,
            store,
            Box::new(Fsrs::default()),
            crate::session::SessionOptions {
                depth,
                ..Default::default()
            },
            now_ms(),
        );
        let mut decks = HashMap::new();
        decks.insert("d.txt".to_string(), deck);
        Reviewing::new(SessionBuild {
            session,
            label: "d.txt".to_string(),
            decks,
            links: HashMap::new(),
            source_roots: HashMap::new(),
            source_bases: HashMap::new(),
            topology_name: None,
        })
    }

    #[test]
    fn state_reports_the_sessions_depth_and_typeline_mode() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("d.txt");
        let text = "# steps\n% reveal: line\n\tfirst\n\tsecond\n";
        std::fs::write(&deck, text).unwrap();
        let cards = crate::parser::parse_str("d.txt", text).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert(cards[0].id(), 0); // seen, so it's a quiz not an acquire
        let r = reviewing_at(deck, cards, &store, Depth::Reconstruct);

        let dto = review_state(Some(&r), &store);
        assert_eq!(
            "reconstruct", dto.depth,
            "the DTO reports the session's depth"
        );
        assert_eq!(
            "typeline", dto.mode,
            "reconstruct + `% reveal: line` types the next line"
        );
    }

    #[test]
    fn recognize_state_offers_gap_options_for_a_cloze_card() {
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("d.txt");
        // A real expanded cloze card (its sub-card's back is the bare gap text)
        // plus sibling cards whose backs are the gap distractors.
        let text = "# where\n% reveal: cloze\n\tThe {{cat}} sat here\n# a\n\tdog\n# b\n\tfish\n# c\n\tbird\n";
        std::fs::write(&deck, text).unwrap();
        let cards = crate::parser::parse_str("d.txt", text).unwrap();
        assert_eq!(vec!["cat".to_string()], cards[0].back); // gap text is the back
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        store.get_or_insert(cards[0].id(), 0); // seen → the Recognize MC, not the acquire on-ramp
        let r = reviewing_at(deck, cards, &store, Depth::Recognize);

        let dto = review_state(Some(&r), &store);
        let opts = dto
            .choices
            .expect("a Recognize cloze card offers gap-filler options");
        assert_eq!(choice::NUM_OPTIONS, opts.len());
        assert!(
            opts.contains(&"cat".to_string()),
            "the gap text is an option"
        );
    }

    #[test]
    fn an_already_recognized_card_skips_the_acquire_mc() {
        // A card recognized in a prior Recognize session carries `recognized_ms`
        // and a store entry, so a later Recall session quizzes it directly — never
        // through the recognition-MC acquire on-ramp (spec §4.6).
        let dir = tempfile::tempdir().unwrap();
        let deck = dir.path().join("d.txt");
        let text = "# q\n\tanswer\n";
        std::fs::write(&deck, text).unwrap();
        let cards = crate::parser::parse_str("d.txt", text).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let state = store.get_or_insert(cards[0].id(), 0);
        state.recognized_ms = Some(500); // recognized, but no Recall schedule yet
        let r = reviewing_at(deck, cards, &store, Depth::Recall);

        let dto = review_state(Some(&r), &store);
        assert!(!dto.acquire, "a recognized card isn't acquired cold");
        assert!(
            dto.choices.is_none(),
            "no recognition MC for an already-recognized card"
        );
        assert_eq!("recall", dto.depth);
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

    #[test]
    fn a_frozen_card_with_no_resolvable_source_root_answers_immediately_without_spawning() {
        // Same condition as ask.rs's `(Some(excerpt), None)` prompt arm (the
        // card is frozen, but its live `% origin:`/deck root doesn't exist on
        // disk) — serve should answer with `SOURCE_NOT_FOUND` synchronously
        // instead of asking the model to echo it. Point the ask config at a
        // nonexistent binary: if the short-circuit works, it's never touched.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("29.rs"), "fn real() {}\n").unwrap();
        let deck_path = dir.path().join("d.txt");
        std::fs::write(
            &deck_path,
            "% source: 29.rs\n# q\n\ta\n\t% at: 29.rs:1 from src/caching.rs:46-66\n",
        )
        .unwrap();
        let deck = crate::deck::Deck::load(&deck_path).unwrap();
        let card = deck.cards[0].clone();
        assert!(card.at_origin.is_some(), "the card is frozen");

        let store = Store::open(dir.path().join("p.json")).unwrap();
        let session = Session::new(
            vec![card.clone()],
            &store,
            Box::new(Fsrs::default()),
            crate::session::SessionOptions::default(),
            now_ms(),
        );
        let mut decks = HashMap::new();
        decks.insert("d.txt".to_string(), deck_path);
        let mut source_roots = HashMap::new();
        // Configured (`source_access` opted in), but unresolved on disk.
        source_roots.insert("d.txt".to_string(), dir.path().join("gone-origin"));
        let mut source_bases = HashMap::new();
        source_bases.insert("d.txt".to_string(), SourceBase::for_deck(&deck));
        let mut r = Reviewing::new(SessionBuild {
            session,
            label: "d.txt".to_string(),
            decks,
            links: HashMap::new(),
            source_roots,
            source_bases,
            topology_name: None,
        });

        let cfg = crate::testutil::ask_config(&dir.path().join("no-such-claude-binary"));
        assert!(r.start_ask(&cfg, Some("why?".to_string())));

        // Answered synchronously: no thread/channel, so nothing is pending, and
        // the reply is already in the transcript on the very next read — the
        // page's first poll (`GET /api/ask`) sees it immediately.
        assert!(r.ask.pending.is_none(), "the backend was never spawned");
        assert_eq!(1, r.ask.transcript.len());
        assert_eq!("why?", r.ask.transcript[0].0);
        assert_eq!(ask::SOURCE_NOT_FOUND, r.ask.transcript[0].1);
        assert!(!r.ask_dto(None, None).thinking, "never stuck thinking");

        let (status, error) = r.poll_ask();
        assert_eq!((None, None), (status, error));
        assert_eq!(1, r.ask.transcript.len(), "poll_ask doesn't double-answer");
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
        assert_eq!(Some("partly"), d.verdict); // machine token, not a display label
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
        store.get_or_insert(deck.cards[0].id(), now).recall = Some(crate::store::FsrsState {
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

    #[test]
    fn a_lan_pairing_reply_carries_a_qr_svg() {
        // Mirrors the `/api/pair` handler's own construction: an SVG only
        // when the pairing info is reachable off-device.
        let pair = PairInfo {
            url: "http://192.168.1.2:7777/?token=ab".to_string(),
            lan: true,
        };
        let svg = if pair.lan {
            crate::qr::svg(&pair.url)
        } else {
            None
        };
        assert!(svg.unwrap().starts_with("<svg "));
    }

    #[test]
    fn a_scoped_instance_always_keeps_its_current_dir() {
        let current = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let cfg = current.path().join("config.toml");
        std::fs::write(
            &cfg,
            format!("decks_dir = \"{}\"\n", other.path().display()),
        )
        .unwrap();
        let dir = effective_decks_dir(true, Some(&cfg), current.path());
        assert_eq!(current.path(), dir);
    }

    #[test]
    fn an_unscoped_instance_follows_a_config_naming_a_different_dir() {
        let current = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let cfg = current.path().join("config.toml");
        std::fs::write(
            &cfg,
            format!("decks_dir = \"{}\"\n", other.path().display()),
        )
        .unwrap();
        let dir = effective_decks_dir(false, Some(&cfg), current.path());
        assert_eq!(other.path(), dir);
    }

    #[test]
    fn an_unparseable_config_keeps_the_current_dir() {
        let current = tempfile::tempdir().unwrap();
        let cfg = current.path().join("config.toml");
        std::fs::write(&cfg, "not valid toml [[[\n").unwrap();
        let dir = effective_decks_dir(false, Some(&cfg), current.path());
        assert_eq!(current.path(), dir);
    }

    /// The JSON-API contract snapshot suite (docs/API.md): every wire-facing
    /// DTO gets its entire serialized shape pinned by full-object equality, so
    /// any field add/remove/rename/retype fails here with a pointer at the
    /// doc. Each pin also emits its expected JSON to `tests/contracts/` — the
    /// machine-readable corpus for thin-client codegen. The page-private
    /// keybinding DTOs (`KeyDto`, `ReviewKeys`, `PickerKeysDto`, `BrowseKeys`)
    /// are deliberately out of contract and unpinned.
    mod contract {
        use serde_json::json;

        use super::*;

        /// Pins a DTO's exact wire shape and emits it to the codegen corpus.
        /// A failure means the JSON contract moved: update docs/API.md's field
        /// table + example for this anchor AND add a CHANGELOG entry.
        fn pin<T: serde::Serialize>(anchor: &str, dto: &T, expected: serde_json::Value) {
            let actual = serde_json::to_value(dto).unwrap();
            assert_eq!(
                actual, expected,
                "wire shape drifted from docs/API.md#{anchor} — update the doc's \
                 field table + example AND add a CHANGELOG entry"
            );
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join(format!("{anchor}.json")),
                serde_json::to_string_pretty(&expected).unwrap() + "\n",
            )
            .unwrap();
        }

        #[test]
        fn statedto_select_phase_wire_shape() {
            let dto = StateDto {
                kind: "review",
                phase: "select",
                card: None,
                choices: None,
                keypoints: None,
                acquire: false,
                mode: "flip",
                depth: "recall",
                input: "type",
                remaining: 0,
                initial: 0,
                reviews: 0,
                passed: 0,
                failed: 0,
                exam_due: Vec::new(),
                can_restart: false,
                promotable: false,
                label: "select decks".to_string(),
            };
            pin(
                "StateDto.select",
                &dto,
                json!({
                    "kind": "review",
                    "phase": "select",
                    "card": null,
                    "choices": null,
                    "keypoints": null,
                    "acquire": false,
                    "mode": "flip",
                    "depth": "recall",
                    "input": "type",
                    "remaining": 0,
                    "initial": 0,
                    "reviews": 0,
                    "passed": 0,
                    "failed": 0,
                    "exam_due": [],
                    "can_restart": false,
                    "promotable": false,
                    "label": "select decks"
                }),
            );
        }

        #[test]
        fn statedto_review_phase_wire_shape() {
            let dto = StateDto {
                kind: "review",
                phase: "review",
                card: Some(CardDto {
                    front: "What is ownership?".to_string(),
                    context: vec!["Chapter 4".to_string()],
                    back: vec!["every value has one owner".to_string()],
                    reshaped: true,
                    note: vec![
                        NoteUnitDto::Sentence {
                            text: "Ownership frees memory deterministically.".to_string(),
                        },
                        NoteUnitDto::Code {
                            lines: vec!["let s = String::new();".to_string()],
                        },
                    ],
                    img: Some("/img/0123456789abcdef".to_string()),
                    img_back: Some("/img/0123456789abcdef".to_string()),
                    at: Some("string.rs:120-128".to_string()),
                    citation: Some(ExcerptDto {
                        path: "src/string.rs".to_string(),
                        lines: vec![LineDto {
                            n: 120,
                            text: "let s = String::new();".to_string(),
                        }],
                        truncated: false,
                    }),
                    citation_error: None,
                    crumb: Some(CrumbDto {
                        regions: vec!["intro".to_string(), "body".to_string()],
                        current: 1,
                        cells: vec![vec![0.5], vec![1.0]],
                    }),
                }),
                choices: Some(vec!["owner".to_string(), "borrower".to_string()]),
                keypoints: Some(vec!["one owner per value".to_string()]),
                acquire: false,
                mode: "flip",
                depth: "recall",
                input: "type",
                remaining: 3,
                initial: 5,
                reviews: 2,
                passed: 1,
                failed: 1,
                exam_due: vec!["rust.txt".to_string()],
                can_restart: true,
                promotable: true,
                label: "rust.txt".to_string(),
            };
            pin(
                "StateDto.review",
                &dto,
                json!({
                    "kind": "review",
                    "phase": "review",
                    "card": {
                        "front": "What is ownership?",
                        "context": ["Chapter 4"],
                        "back": ["every value has one owner"],
                        "reshaped": true,
                        "note": [
                            {"kind": "sentence", "text": "Ownership frees memory deterministically."},
                            {"kind": "code", "lines": ["let s = String::new();"]}
                        ],
                        "img": "/img/0123456789abcdef",
                        "img_back": "/img/0123456789abcdef",
                        "at": "string.rs:120-128",
                        "citation": {
                            "path": "src/string.rs",
                            "lines": [{"n": 120, "text": "let s = String::new();"}],
                            "truncated": false
                        },
                        "citation_error": null,
                        "crumb": {
                            "regions": ["intro", "body"],
                            "current": 1,
                            "cells": [[0.5], [1.0]]
                        }
                    },
                    "choices": ["owner", "borrower"],
                    "keypoints": ["one owner per value"],
                    "acquire": false,
                    "mode": "flip",
                    "depth": "recall",
                    "input": "type",
                    "remaining": 3,
                    "initial": 5,
                    "reviews": 2,
                    "passed": 1,
                    "failed": 1,
                    "exam_due": ["rust.txt"],
                    "can_restart": true,
                    "promotable": true,
                    "label": "rust.txt"
                }),
            );
        }

        #[test]
        fn walkdto_predict_phase_wire_shape() {
            let dto = WalkDto {
                kind: "walk",
                phase: "predict",
                description: "how a String grows".to_string(),
                source: Some("src/lib.rs".to_string()),
                total: 2,
                current: 2,
                path: vec![
                    HopDto {
                        prompt: "push begins".to_string(),
                        delta: Some("passed"),
                        current: false,
                    },
                    HopDto {
                        prompt: "capacity doubles".to_string(),
                        delta: None,
                        current: true,
                    },
                ],
                prompt: Some("what does push do when full?".to_string()),
                givens: vec!["a String at capacity".to_string()],
                locator: Some("lib.rs:40-52".to_string()),
                prediction: None,
                excerpt: None,
                excerpt_error: None,
                points: Vec::new(),
                note: None,
                auto_grade: false,
                thinking: false,
                verdict: None,
                feedback: None,
                grade_error: None,
                summary: None,
            };
            pin(
                "WalkDto.predict",
                &dto,
                json!({
                    "kind": "walk",
                    "phase": "predict",
                    "description": "how a String grows",
                    "source": "src/lib.rs",
                    "total": 2,
                    "current": 2,
                    "path": [
                        {"prompt": "push begins", "delta": "passed", "current": false},
                        {"prompt": "capacity doubles", "delta": null, "current": true}
                    ],
                    "prompt": "what does push do when full?",
                    "givens": ["a String at capacity"],
                    "locator": "lib.rs:40-52",
                    "prediction": null,
                    "excerpt": null,
                    "excerpt_error": null,
                    "points": [],
                    "note": null,
                    "auto_grade": false,
                    "thinking": false,
                    "verdict": null,
                    "feedback": null,
                    "grade_error": null,
                    "summary": null
                }),
            );
        }

        #[test]
        fn walkdto_done_phase_wire_shape() {
            let dto = WalkDto {
                kind: "walk",
                phase: "done",
                description: "how a String grows".to_string(),
                source: None,
                total: 3,
                current: 3,
                path: vec![HopDto {
                    prompt: "capacity doubles".to_string(),
                    delta: Some("partly"),
                    current: false,
                }],
                prompt: None,
                givens: Vec::new(),
                locator: None,
                prediction: Some("it reallocates".to_string()),
                excerpt: Some(ExcerptDto {
                    path: "src/lib.rs".to_string(),
                    lines: vec![LineDto {
                        n: 40,
                        text: "self.grow();".to_string(),
                    }],
                    truncated: true,
                }),
                excerpt_error: None,
                points: vec!["amortized doubling".to_string()],
                note: Some("see also Vec".to_string()),
                auto_grade: true,
                thinking: false,
                verdict: Some("partly"),
                feedback: Some("half right".to_string()),
                grade_error: None,
                summary: Some(SummaryDto {
                    passed: 1,
                    partly: 1,
                    failed: 1,
                    weak: vec![2, 3],
                    total: 3,
                }),
            };
            pin(
                "WalkDto.done",
                &dto,
                json!({
                    "kind": "walk",
                    "phase": "done",
                    "description": "how a String grows",
                    "source": null,
                    "total": 3,
                    "current": 3,
                    "path": [
                        {"prompt": "capacity doubles", "delta": "partly", "current": false}
                    ],
                    "prompt": null,
                    "givens": [],
                    "locator": null,
                    "prediction": "it reallocates",
                    "excerpt": {
                        "path": "src/lib.rs",
                        "lines": [{"n": 40, "text": "self.grow();"}],
                        "truncated": true
                    },
                    "excerpt_error": null,
                    "points": ["amortized doubling"],
                    "note": "see also Vec",
                    "auto_grade": true,
                    "thinking": false,
                    "verdict": "partly",
                    "feedback": "half right",
                    "grade_error": null,
                    "summary": {"passed": 1, "partly": 1, "failed": 1, "weak": [2, 3], "total": 3}
                }),
            );
        }

        #[test]
        fn examdto_results_phase_wire_shape() {
            let dto = ExamDto {
                phase: "results",
                deck: "rust.txt".to_string(),
                strictness: "balanced",
                total: 1,
                current: 1,
                question: None,
                answer: String::new(),
                on_last: true,
                grades: vec![ExamGradeDto {
                    question: "Why does Rust use ownership?".to_string(),
                    points: vec!["memory safety without a GC".to_string()],
                    answer: "it frees memory deterministically".to_string(),
                    verdict: "PASS",
                    feedback: "solid".to_string(),
                    missed: Vec::new(),
                }],
                passed: Some(true),
                gaps: Vec::new(),
                can_remediate: false,
                remediated_count: None,
                is_trace: false,
                unlocks: vec!["next.txt".to_string()],
                thinking: false,
                error: None,
                elapsed: None,
                cooldown_ms: None,
            };
            pin(
                "ExamDto.results",
                &dto,
                json!({
                    "phase": "results",
                    "deck": "rust.txt",
                    "strictness": "balanced",
                    "total": 1,
                    "current": 1,
                    "question": null,
                    "answer": "",
                    "on_last": true,
                    "grades": [{
                        "question": "Why does Rust use ownership?",
                        "points": ["memory safety without a GC"],
                        "answer": "it frees memory deterministically",
                        "verdict": "PASS",
                        "feedback": "solid",
                        "missed": []
                    }],
                    "passed": true,
                    "gaps": [],
                    "can_remediate": false,
                    "remediated_count": null,
                    "is_trace": false,
                    "unlocks": ["next.txt"],
                    "thinking": false,
                    "error": null,
                    "elapsed": null,
                    "cooldown_ms": null
                }),
            );
        }

        #[test]
        fn examdto_cooldown_phase_wire_shape() {
            let dto = cooldown_dto("deck.txt", 90000);
            pin(
                "ExamDto.cooldown",
                &dto,
                json!({
                    "phase": "cooldown",
                    "deck": "deck.txt",
                    "strictness": "balanced",
                    "total": 0,
                    "current": 0,
                    "question": null,
                    "answer": "",
                    "on_last": false,
                    "grades": [],
                    "passed": null,
                    "gaps": [],
                    "can_remediate": false,
                    "remediated_count": null,
                    "is_trace": true,
                    "unlocks": [],
                    "thinking": false,
                    "error": null,
                    "elapsed": null,
                    "cooldown_ms": 90000
                }),
            );
        }

        #[test]
        fn decklistdto_wire_shape() {
            let dto = DeckListDto {
                workspaces: vec![DeckItemDto {
                    name: "rustws".to_string(),
                    label: "Rust workspace".to_string(),
                    meta: Some("3/10".to_string()),
                    state: "workspace",
                    locked: false,
                    reviewable: true,
                    reviewable_recognize: false,
                    reviewable_recall: true,
                    reviewable_reconstruct: true,
                    mastered: false,
                    is_trace: false,
                    examable: false,
                    has_exam: false,
                    recent: true,
                    is_workspace: true,
                    description: Some("learn Rust ownership".to_string()),
                    members: vec![MemberDto {
                        name: "rustws/intro.txt".to_string(),
                        label: "Intro".to_string(),
                        meta: Some("3/10".to_string()),
                        state: "started",
                        locked: false,
                        reviewable: true,
                        reviewable_recognize: true,
                        reviewable_recall: true,
                        reviewable_reconstruct: false,
                        mastered: false,
                        is_trace: false,
                        examable: true,
                        has_exam: true,
                        indent: 1,
                        tree: "└─ ".to_string(),
                        has_topology: false,
                        badge_depth: None,
                        badge_dotted: false,
                        new_cards: false,
                        last_depth: "recall",
                    }],
                    path: Some("~/decks".to_string()),
                    icon: Some("/img/0123456789abcdef".to_string()),
                    icon_svg: true,
                    has_topology: true,
                    badge_depth: Some("recall"),
                    badge_dotted: true,
                    new_cards: true,
                    last_depth: "recall",
                }],
                recent: Vec::new(),
                folders: Vec::new(),
            };
            pin(
                "DeckListDto",
                &dto,
                json!({
                    "workspaces": [{
                        "name": "rustws",
                        "label": "Rust workspace",
                        "meta": "3/10",
                        "state": "workspace",
                        "locked": false,
                        "reviewable": true,
                        "reviewable_recognize": false,
                        "reviewable_recall": true,
                        "reviewable_reconstruct": true,
                        "mastered": false,
                        "is_trace": false,
                        "examable": false,
                        "has_exam": false,
                        "recent": true,
                        "is_workspace": true,
                        "description": "learn Rust ownership",
                        "members": [{
                            "name": "rustws/intro.txt",
                            "label": "Intro",
                            "meta": "3/10",
                            "state": "started",
                            "locked": false,
                            "reviewable": true,
                            "reviewable_recognize": true,
                            "reviewable_recall": true,
                            "reviewable_reconstruct": false,
                            "mastered": false,
                            "is_trace": false,
                            "examable": true,
                            "has_exam": true,
                            "indent": 1,
                            "tree": "└─ ",
                            "has_topology": false,
                            "badge_depth": null,
                            "badge_dotted": false,
                            "new_cards": false,
                            "last_depth": "recall"
                        }],
                        "path": "~/decks",
                        "icon": "/img/0123456789abcdef",
                        "icon_svg": true,
                        "has_topology": true,
                        "badge_depth": "recall",
                        "badge_dotted": true,
                        "new_cards": true,
                        "last_depth": "recall"
                    }],
                    "recent": [],
                    "folders": []
                }),
            );
        }

        #[test]
        fn decktopologydto_wire_shape() {
            let dto = DeckTopologyDto {
                topologies: vec![TopologyInfoDto {
                    name: "north-south".to_string(),
                    principle: "north to south".to_string(),
                    regions: vec![RegionInfoDto {
                        name: "north".to_string(),
                        cells: vec![0.5, 1.0],
                        due: 2,
                    }],
                }],
                deck_due: 3,
            };
            pin(
                "DeckTopologyDto",
                &dto,
                json!({
                    "topologies": [{
                        "name": "north-south",
                        "principle": "north to south",
                        "regions": [{"name": "north", "cells": [0.5, 1.0], "due": 2}]
                    }],
                    "deck_due": 3
                }),
            );
        }

        #[test]
        fn browsedto_wire_shape() {
            let dto = BrowseDto {
                phase: "browse",
                label: "rust.txt".to_string(),
                cards: vec![CardDto {
                    front: "q".to_string(),
                    context: Vec::new(),
                    back: vec!["a".to_string()],
                    reshaped: false,
                    note: Vec::new(),
                    img: None,
                    img_back: None,
                    at: None,
                    citation: None,
                    citation_error: None,
                    crumb: None,
                }],
            };
            pin(
                "BrowseDto",
                &dto,
                json!({
                    "phase": "browse",
                    "label": "rust.txt",
                    "cards": [{
                        "front": "q",
                        "context": [],
                        "back": ["a"],
                        "reshaped": false,
                        "note": [],
                        "img": null,
                        "img_back": null,
                        "at": null,
                        "citation": null,
                        "citation_error": null,
                        "crumb": null
                    }]
                }),
            );
        }

        #[test]
        fn choosefeedbackdto_wire_shape() {
            let dto = ChooseFeedbackDto {
                chosen: 2,
                correct: 1,
                passed: false,
            };
            pin(
                "ChooseFeedbackDto",
                &dto,
                json!({"chosen": 2, "correct": 1, "passed": false}),
            );
        }

        #[test]
        fn checkfeedbackdto_wire_shape() {
            let dto = CheckFeedbackDto {
                results: vec![TypedResult {
                    input: "pars".to_string(),
                    expected: "Paris".to_string(),
                    passed: false,
                }],
                passed: false,
            };
            pin(
                "CheckFeedbackDto",
                &dto,
                json!({
                    "results": [{"input": "pars", "expected": "Paris", "passed": false}],
                    "passed": false
                }),
            );
        }

        #[test]
        fn askdto_populated_wire_shape() {
            let dto = AskDto {
                transcript: vec![ExchangeDto {
                    q: "why one owner?".to_string(),
                    a: "so drops are deterministic".to_string(),
                }],
                thinking: true,
                status: Some("asking claude".to_string()),
                error: None,
            };
            pin(
                "AskDto.populated",
                &dto,
                json!({
                    "transcript": [{"q": "why one owner?", "a": "so drops are deterministic"}],
                    "thinking": true,
                    "status": "asking claude",
                    "error": null
                }),
            );
        }

        #[test]
        fn askdto_empty_wire_shape() {
            let dto = AskDto {
                transcript: Vec::new(),
                thinking: false,
                status: None,
                error: None,
            };
            pin(
                "AskDto.empty",
                &dto,
                json!({
                    "transcript": [],
                    "thinking": false,
                    "status": null,
                    "error": null
                }),
            );
        }

        #[test]
        fn askinfodto_and_versiondto_wire_shape() {
            let info = AskInfoDto {
                model: "default".to_string(),
                effort: "default".to_string(),
            };
            pin(
                "AskInfoDto",
                &info,
                json!({"model": "default", "effort": "default"}),
            );
            let version = VersionDto {
                version: env!("CARGO_PKG_VERSION"),
            };
            pin(
                "VersionDto",
                &version,
                json!({"version": env!("CARGO_PKG_VERSION")}),
            );
        }

        #[test]
        fn doctordto_wire_shape() {
            let dto = DoctorDto {
                rows: vec![
                    DoctorRowDto {
                        name: "config",
                        status: "ok",
                        detail: "~/.config/alix/config.toml parses".to_string(),
                        remedy: None,
                    },
                    DoctorRowDto {
                        name: "share",
                        status: "warn",
                        detail: "`wormhole` not found on PATH".to_string(),
                        remedy: Some("pipx install magic-wormhole".to_string()),
                    },
                ],
            };
            pin(
                "DoctorDto",
                &dto,
                json!({
                    "rows": [
                        {"name": "config", "status": "ok",
                         "detail": "~/.config/alix/config.toml parses", "remedy": null},
                        {"name": "share", "status": "warn",
                         "detail": "`wormhole` not found on PATH",
                         "remedy": "pipx install magic-wormhole"}
                    ]
                }),
            );
        }

        #[test]
        fn pairdto_wire_shape() {
            let dto = PairDto {
                url: "http://127.0.0.1:7777/".to_string(),
                svg: None,
                lan: false,
            };
            pin(
                "PairDto",
                &dto,
                json!({"url": "http://127.0.0.1:7777/", "svg": null, "lan": false}),
            );
        }

        #[test]
        fn resetdto_wire_shape() {
            let dto = ResetDto {
                deck: "rust.txt".to_string(),
                cards_cleared: 17,
            };
            pin(
                "ResetDto",
                &dto,
                json!({"deck": "rust.txt", "cards_cleared": 17}),
            );
        }

        #[test]
        fn importdto_wire_shape() {
            let dto = ImportDto {
                deck: "kanji.txt".to_string(),
                cards: 40,
            };
            pin("ImportDto", &dto, json!({"deck": "kanji.txt", "cards": 40}));
        }

        #[test]
        fn augmentdto_wire_shape() {
            let dto = AugmentDto {
                deck: "rust.txt".to_string(),
                cards: 12,
                rows: vec![AugmentRowDto {
                    kind: "choices",
                    label: "choice distractors",
                    covered: 4,
                    eligible: 12,
                    items: vec!["north-south".to_string()],
                    busy: true,
                }],
                busy: Some("choices"),
                elapsed: Some(3),
                error: None,
            };
            pin(
                "AugmentDto",
                &dto,
                json!({
                    "deck": "rust.txt",
                    "cards": 12,
                    "rows": [{
                        "kind": "choices",
                        "label": "choice distractors",
                        "covered": 4,
                        "eligible": 12,
                        "items": ["north-south"],
                        "busy": true
                    }],
                    "busy": "choices",
                    "elapsed": 3,
                    "error": null
                }),
            );
        }

        #[test]
        fn generatedto_done_wire_shape() {
            let dto = GenerateDto {
                phase: "done",
                deck: Some("rust-ownership.txt".to_string()),
                cards: Some(12),
                elapsed: Some(41),
                error: None,
            };
            pin(
                "GenerateDto",
                &dto,
                json!({"phase": "done", "deck": "rust-ownership.txt", "cards": 12,
                       "elapsed": 41, "error": null}),
            );
        }

        #[test]
        fn sharedto_code_phase_wire_shape() {
            let dto = ShareDto {
                phase: "code",
                code: Some("7-alpha-bravo".to_string()),
                elapsed: Some(3),
                error: None,
            };
            pin(
                "ShareDto",
                &dto,
                json!({"phase": "code", "code": "7-alpha-bravo", "elapsed": 3, "error": null}),
            );
        }

        #[test]
        fn receivedto_done_wire_shape() {
            let dto = ReceiveDto {
                phase: "done",
                landed: Some("rust-decks".to_string()),
                stripped: vec!["progress.json".to_string()],
                elapsed: Some(9),
                error: None,
            };
            pin(
                "ReceiveDto",
                &dto,
                json!({"phase": "done", "landed": "rust-decks",
                       "stripped": ["progress.json"], "elapsed": 9, "error": null}),
            );
        }
    }
}
