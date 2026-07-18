//! The wire shapes — every DTO the JSON API serializes, their builders, and
//! the name mappers. Private to the serve module (`pub(super)`), pinned by
//! the contract tests.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use serde::{Deserialize, Serialize};

use super::{Browsing, Examining, Reviewing, Walking, catalog::img_key};
use crate::{
    answer::{Input, Mode, mode_name},
    augment::AugmentCache,
    card::Card,
    config::{
        AskConfig, Bindings, BrowseBindings, Key, KeyPattern, PickerKeys, ReviewConfig, Strictness,
    },
    deck::{self, Deck, DeckState},
    depth::{Depth, depth_name},
    doctor, exam,
    render::NoteUnit,
    review::{self, CardView},
    scheduler::Fsrs,
    session::now_ms,
    store::Store,
    trace::{self, Delta, Excerpt, Phase},
};

/// A card serialized for the browser.
#[derive(Debug, Serialize)]
pub(super) struct CardDto {
    pub(super) front: String,
    pub(super) context: Vec<String>,
    pub(super) back: Vec<String>,
    /// True when `back` is a reshaped answer (a `format` augment's `display_back`),
    /// so the frontend bullets a multi-line list. Never set for the card's own
    /// authored back lines (a poem, typing answers) — only the reshape.
    pub(super) reshaped: bool,
    /// The core note units serialize as the documented wire shape directly;
    /// the web page renders `sentence` as a paragraph and `code` verbatim.
    pub(super) note: Vec<NoteUnit>,
    /// `/img/<key>` URL for the question-side image, or `null`.
    pub(super) img: Option<String>,
    /// `/img/<key>` URL for the answer-side image, shown on reveal, or `null`.
    pub(super) img_back: Option<String>,
    /// The card's `% at:` source citation locator (e.g. `string.rs:120-128`),
    /// shown compact on reveal and expandable to `citation`. `null` if absent.
    pub(super) at: Option<String>,
    /// The resolved excerpt for `at`, expanded in the browser on demand. `null`
    /// when the card has no `% at:` or it couldn't be resolved.
    pub(super) citation: Option<ExcerptDto>,
    /// Why `at` couldn't be resolved (missing file, drifted line range), if it
    /// failed — shown dim in place of the excerpt.
    pub(super) citation_error: Option<String>,
    /// The topological orientation breadcrumb — coarse region names with the
    /// current one marked — when the session is topology-ordered. `null`
    /// otherwise. Resolved by `review_state`, which holds the topology.
    pub(super) crumb: Option<CrumbDto>,
}

/// The "where am I" region breadcrumb: the topology's region names in walk order
/// and the index of the current card's region. The page windows this to its
/// width (worst case: previous, current, next).
#[derive(Debug, Serialize)]
pub(super) struct CrumbDto {
    pub(super) regions: Vec<String>,
    pub(super) current: usize,
    /// Per-region, per-card strength (`0..=1`, outer index aligns with
    /// `regions`) for the heatmap bar under each region — each card a cell,
    /// red (weak) → green (strong).
    pub(super) cells: Vec<Vec<f32>>,
}

/// A deck's stored topologies with their region heatmaps, fetched on demand for
/// the picker's pre-launch **focus drawer** (`/api/deck-topology`): choose a
/// topology, see each region's strength, tap one to drill it.
#[derive(Debug, Serialize, Default)]
pub(super) struct DeckTopologyDto {
    pub(super) topologies: Vec<TopologyInfoDto>,
    /// Cards due/new across the whole deck right now — the count shown for the
    /// drawer's "Whole deck" option.
    pub(super) deck_due: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct TopologyInfoDto {
    pub(super) name: String,
    /// The one-line ordering principle (e.g. "north to south"), shown beside the
    /// name in the drawer's topology picker so several are told apart.
    pub(super) principle: String,
    pub(super) regions: Vec<RegionInfoDto>,
}

#[derive(Debug, Serialize)]
pub(super) struct RegionInfoDto {
    pub(super) name: String,
    /// Per-card strength (`0..=1`), red → green — the region's heatmap bar.
    pub(super) cells: Vec<f32>,
    /// Cards due/new in this region right now — shown when it's the selection.
    pub(super) due: usize,
}

/// The current review state sent to the browser after every action.
#[derive(Debug, Serialize)]
pub(super) struct StateDto {
    /// Discriminates this payload from the trace-walk [`WalkDto`] for the single
    /// client dispatcher (`isWalk`): always `"review"` here.
    pub(super) kind: &'static str,
    /// `"select"` while choosing decks (no session yet), `"review"` mid-session,
    /// `"done"` once nothing is left — the session-end signal. There is no
    /// separate `finished` flag; a finished session is just the `done` phase
    /// (matching the walk's own `done`).
    pub(super) phase: &'static str,
    /// The card up for review, or `null` when finished or in the select phase.
    pub(super) card: Option<CardDto>,
    /// For `choice` mode, the multiple-choice options (one is correct); `null`
    /// otherwise, or when the card has too few distractors (the page then
    /// falls back to reveal). The correct index is never sent here.
    pub(super) choices: Option<Vec<String>>,
    /// For `explain` mode past acquire, the rubric the reveal checks a
    /// reconstruction against (each ticked ✓/✗, the coverage deriving the
    /// grade): the cached key points when present, else the card's own back
    /// lines. `null` for any other mode.
    pub(super) keypoints: Option<Vec<String>>,
    /// Whether the current card has never been seen and should be *acquired*
    /// (shown, then acknowledged with one key) rather than quizzed cold. The page
    /// shows the answer with a single "Seen" button and no grade controls.
    pub(super) acquire: bool,
    /// The answer mode name (`flip`, `line`, `typeline`, …); the page reveals
    /// line-by-line for `line` and flip-style otherwise. Derived from the card's
    /// `% reveal:` and the session's `depth`.
    pub(super) mode: &'static str,
    /// The session's chosen depth (`recognize` / `recall` / `reconstruct`) — the
    /// depth of practice this session runs at (spec §4).
    pub(super) depth: &'static str,
    /// The input method (`type` / `draw`). `draw` tells the page to show the
    /// canvas for a self-graded card; orthogonal to `mode`. The runtime "Draw
    /// answers" toggle lives in the browser and never appears here.
    pub(super) input: &'static str,
    pub(super) remaining: u32,
    pub(super) initial: u32,
    pub(super) reviews: u32,
    pub(super) passed: u32,
    pub(super) failed: u32,
    /// Never-seen cards introduced this session; a first pass over a fresh
    /// deck is acquire-only, and the summary must say so instead of "0".
    pub(super) acquired: u32,
    /// Subjects of decks in this (finished) session that are now `ExamDue` —
    /// drilled, sourced, and not yet mastered. The summary offers to sit each.
    /// Empty until the session is finished.
    pub(super) exam_due: Vec<String>,
    /// Whether a restart would find any due/new cards right now. The summary
    /// disables "New session" and shows a "nothing due" note when this is
    /// false.
    pub(super) can_restart: bool,
    /// Whether the current card is a virtual (remediation) card, so the page
    /// can offer to promote it into its deck file (appends it as a deck card,
    /// carrying over its review schedule rather than starting fresh). `false`
    /// once nothing is current, or for an authored deck card.
    pub(super) promotable: bool,
    pub(super) label: String,
}

/// The payload of the browse view: every (remaining) card, in deck order, or
/// an empty list in the `select` phase.
#[derive(Debug, Serialize)]
pub(super) struct BrowseDto {
    /// `"select"` while choosing decks, else `"browse"`.
    pub(super) phase: &'static str,
    pub(super) label: String,
    pub(super) cards: Vec<CardDto>,
}

/// The deck-selection catalog sent to the browser picker, in three sections:
/// `workspaces` (each with its last-progress time), `recent` loose decks
/// (recent-first), and plain `folders`. A deck inside a
/// workspace stays out of `recent` — you reach it by opening the workspace. The
/// filter searches every loose deck.
#[derive(Debug, Serialize)]
pub(super) struct DeckListDto {
    pub(super) workspaces: Vec<DeckItemDto>,
    pub(super) recent: Vec<DeckItemDto>,
    pub(super) folders: Vec<DeckItemDto>,
}

/// One row in the selection screen: its selection `name`, a display `label`, a
/// completion-state badge (`new` / `m/total` / `done ✓` / `mastered 🎉`), a
/// machine-readable `state` for styling, and the flags the picker dims/glyphs a
/// row with — `locked` (🔒, a `% requires:` prerequisite), `reviewable` (false →
/// 🕒, nothing due), `mastered` (🎉, tucked into the Mastered window).
#[derive(Debug, Serialize)]
pub(super) struct DeckItemDto {
    /// Stable selection key (file/folder name) sent back on select.
    pub(super) name: String,
    /// STRUCTURAL: whether `name` is the kind of row `/api/select` accepts (a
    /// deck, including one that fails to parse) — `false` for a
    /// workspace/folder group row. Unlike `reviewable*` (state), this never
    /// changes with progress.
    pub(super) selectable: bool,
    /// Display title (`% title:`, else the name without `.txt`, else folder).
    pub(super) label: String,
    pub(super) meta: Option<String>,
    /// `new`/`started`/`finished`/`examdue` for a deck; `workspace`/`folder` for
    /// a drillable row.
    pub(super) state: &'static str,
    pub(super) locked: bool,
    /// Launching now would have something to do; `false` → nothing due (🕒).
    pub(super) reviewable: bool,
    /// A still-unrecognized card that is also recognizable is servable at
    /// Recognize now — see [`picker::DeckStatus::reviewable_recognize`].
    pub(super) reviewable_recognize: bool,
    /// The deck has at least one recognizable card (cached distractors build a
    /// pick) — the client gates the Recognize depth on this, so an un-augmented
    /// deck greys it out even under cram. See
    /// [`picker::DeckStatus::can_recognize`].
    pub(super) can_recognize: bool,
    /// A card is due (or fresh), or a virtual card is due, at Recall — see
    /// [`picker::DeckStatus::reviewable_recall`].
    pub(super) reviewable_recall: bool,
    /// A card is due at Reconstruct — see
    /// [`picker::DeckStatus::reviewable_reconstruct`].
    pub(super) reviewable_reconstruct: bool,
    /// Finished *and* exam-passed — lives in the Mastered window, not Recent.
    pub(super) mastered: bool,
    /// A trace deck (`% trace:`): walked, not card-reviewed.
    pub(super) is_trace: bool,
    /// The AI exam can be sat now (has a `% source:`, not locked) — drilled or
    /// not, so the picker can offer "Take exam" to test out early.
    pub(super) examable: bool,
    /// The deck *has* an exam at all (sourced, non-trace) even if it's locked —
    /// lets the footer always show a "Take exam" control, disabled when locked.
    pub(super) has_exam: bool,
    /// `true` when this entry has recent-use history (shown in Recent by
    /// default; the rest are reachable through the filter).
    pub(super) recent: bool,
    /// `true` for a workspace/folder row (opens into its members on click).
    pub(super) is_workspace: bool,
    /// A workspace's one-line description (its learning goal), shown dim under
    /// the row; `null` for decks and folders.
    pub(super) description: Option<String>,
    /// For a workspace/folder row: its member decks as an unlock dependency
    /// tree, shown when you open it.
    pub(super) members: Vec<MemberDto>,
    /// Dim location hint (parent dir) for entries outside the decks dir; `null`
    /// keeps the row clean. Disambiguates same-named decks/workspaces.
    pub(super) path: Option<String>,
    /// The `/img/<key>` URL of the workspace's icon, or `null` for the chevron.
    pub(super) icon: Option<String>,
    /// `true` when `icon` is an SVG (rendered as a theme-tinted mask); a raster
    /// icon renders as a plain `<img>`.
    pub(super) icon_svg: bool,
    /// `true` when the deck has a cached topology, so the picker's focus drawer
    /// would open for it — the row shows a small drawer indicator.
    pub(super) has_topology: bool,
    /// The highest depth with a badge to show (`"reconstruct"` / `"recall"` /
    /// `"recognize"`), or `null` for none yet — see [`picker::DeckStatus::badge_depth`].
    /// Additive telemetry; gates nothing.
    pub(super) badge_depth: Option<&'static str>,
    /// `true` when `badge_depth`'s badge lapsed (earned once, not currently
    /// solid) and should render dotted rather than solid.
    pub(super) badge_dotted: bool,
    /// Any deck card has no store entry yet — fresh material, distinct from
    /// `state`/`meta`.
    pub(super) new_cards: bool,
    /// The learner's last-used session depth for this deck (`"recognize"` /
    /// `"recall"` / `"reconstruct"`), defaulting to `"recall"`.
    pub(super) last_depth: &'static str,
    /// A workspace's deadline readout ({#deadlines}): present only when the
    /// workspace's `alix.local.toml` sets one. `null` for deck and folder
    /// rows, and for a workspace with no deadline set.
    pub(super) deadline: Option<DeadlineDto>,
}

/// A workspace row's deadline readout ({#deadlines}): present only when the
/// workspace's `alix.local.toml` sets one. `days_left` goes negative past the
/// date (the client renders "was due").
#[derive(Debug, Serialize)]
pub(super) struct DeadlineDto {
    pub(super) date: String, // ISO YYYY-MM-DD
    pub(super) days_left: i64,
    /// Mastered (or done source-less) member decks.
    pub(super) ready: usize,
    /// Member decks.
    pub(super) total: usize,
}

/// A workspace member deck in the drill-in list: a qualified selection `name`
/// (`<workspace>/<file>`), its display `label` and status (badge/state/locked/
/// reviewable/mastered/trace, from the workspace's own store), and its `indent`
/// in the unlock dependency tree (0 = a foundation root).
#[derive(Debug, Serialize)]
pub(super) struct MemberDto {
    pub(super) name: String,
    /// STRUCTURAL — see [`DeckItemDto::selectable`]; always `true` here (a
    /// member row is always a deck file, never a group).
    pub(super) selectable: bool,
    pub(super) label: String,
    pub(super) meta: Option<String>,
    pub(super) state: &'static str,
    pub(super) locked: bool,
    pub(super) reviewable: bool,
    /// Per-depth due-ness — see [`DeckItemDto::reviewable_recognize`] /
    /// [`DeckItemDto::reviewable_recall`] / [`DeckItemDto::reviewable_reconstruct`].
    pub(super) reviewable_recognize: bool,
    /// The member has a recognizable card — see [`DeckItemDto::can_recognize`].
    pub(super) can_recognize: bool,
    pub(super) reviewable_recall: bool,
    pub(super) reviewable_reconstruct: bool,
    pub(super) mastered: bool,
    pub(super) is_trace: bool,
    pub(super) examable: bool,
    pub(super) has_exam: bool,
    pub(super) indent: usize,
    /// The `├─`/`└─`/`│` tree-branch prefix drawn before the label, so a
    /// member's dependency chain reads as a tree, not just indentation.
    pub(super) tree: String,
    /// `true` when the member deck has a cached topology (a focus drawer).
    pub(super) has_topology: bool,
    /// The highest depth with a badge to show, from the workspace's own store —
    /// same semantics as [`DeckItemDto::badge_depth`].
    pub(super) badge_depth: Option<&'static str>,
    /// `true` when `badge_depth`'s badge lapsed (renders dotted, not solid).
    pub(super) badge_dotted: bool,
    /// Any member-deck card has no store entry yet — fresh material.
    pub(super) new_cards: bool,
    /// The learner's last-used session depth for this member deck, defaulting
    /// to `"recall"` — what a plain Learn launches.
    pub(super) last_depth: &'static str,
}

/// One ask-Claude exchange, for the browser.
#[derive(Debug, Serialize)]
pub(super) struct ExchangeDto {
    pub(super) q: String,
    pub(super) a: String,
}

/// The ask-Claude view state: the conversation so far, whether a call is in
/// flight (the page polls while `thinking`), and a transient status / error.
#[derive(Debug, Serialize)]
pub(super) struct AskDto {
    pub(super) transcript: Vec<ExchangeDto>,
    pub(super) thinking: bool,
    pub(super) status: Option<String>,
    pub(super) error: Option<String>,
    /// The last card drafted from the conversation (`AskAction::DraftCard`),
    /// until the subject changes.
    pub(super) draft: Option<DraftCardDto>,
}

/// A card the tutor drafted from the conversation, before the learner edits it.
#[derive(Debug, Clone, Serialize)]
pub(super) struct DraftCardDto {
    pub(super) front: String,
    pub(super) back: Vec<String>,
}

/// The learner's edited draft, posted to mint it as a free-standing card.
#[derive(Debug, Deserialize)]
pub(super) struct CreateCardReq {
    pub(super) front: String,
    pub(super) back: Vec<String>,
}

/// The newly minted virtual card's id.
#[derive(Debug, Serialize)]
pub(super) struct CreateCardResp {
    /// Decimal string (ids are `u64`), matching how the store keys ids.
    pub(super) id: String,
}

/// The ask-tutor's backend, model and effort, shown in the panel so it's clear
/// who is answering — and that it's the CLI default (`"default"`) unless the
/// `[ask]` config pins one, not the stronger model that built the deck. The
/// backend name also feeds every "X is working…" progress line, so a Copilot
/// user never reads "Claude".
#[derive(Debug, Serialize)]
pub(super) struct AskInfoDto {
    /// The configured backend's canonical lowercase name
    /// (`claude` | `gemini` | `codex` | `copilot`).
    pub(super) backend: &'static str,
    pub(super) model: String,
    pub(super) effort: String,
}

/// The running alix version, for the picker's About sheet.
#[derive(Serialize)]
pub(super) struct VersionDto {
    pub(super) version: &'static str,
}

/// The web doctor report (`GET /api/doctor`) — the CLI's free checks,
/// serialized. Costed backend probes stay CLI-only.
#[derive(Serialize)]
pub(super) struct DoctorDto {
    pub(super) rows: Vec<DoctorRowDto>,
}

#[derive(Serialize)]
pub(super) struct DoctorRowDto {
    pub(super) name: &'static str,
    /// `ok` | `warn` | `fail` (open set).
    pub(super) status: &'static str,
    pub(super) detail: String,
    pub(super) remedy: Option<String>,
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
pub(super) struct PairDto {
    pub(super) url: String,
    /// A server-rendered QR of `url`; `null` when this is a localhost-only
    /// instance (nothing another device could reach).
    pub(super) svg: Option<String>,
    pub(super) lan: bool,
}

/// The result of `POST /api/reset`: the row's resolved display name and how
/// many card schedules it wiped.
#[derive(Serialize)]
pub(super) struct ResetDto {
    pub(super) deck: String,
    pub(super) cards_cleared: usize,
}

/// The result of `POST /api/import`: the placed file's name and its card
/// count. Unlike `generate`'s lenient save, an upload that doesn't parse is
/// rejected outright — see the `/api/import` handler.
#[derive(Serialize)]
pub(super) struct ImportDto {
    pub(super) deck: String,
    pub(super) cards: usize,
}

impl AskInfoDto {
    pub(super) fn from(cfg: &AskConfig) -> Self {
        let or_default = |s: &Option<String>| s.clone().unwrap_or_else(|| "default".to_string());
        Self {
            backend: cfg.backend.name(),
            model: or_default(&cfg.model),
            effort: or_default(&cfg.effort),
        }
    }
}

/// One configured key, as the browser sees it: `k` is the `KeyboardEvent.key`
/// value (`" "`, `"Enter"`, `"j"`, …) and `ctrl` whether Ctrl must be held.
#[derive(Debug, Serialize)]
pub(super) struct KeyDto {
    k: String,
    ctrl: bool,
}

pub(super) fn key_dto(p: &KeyPattern) -> KeyDto {
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

pub(super) fn key_list(list: &[KeyPattern]) -> Vec<KeyDto> {
    list.iter().map(key_dto).collect()
}

/// The review actions the web page binds, mirroring the configured `[keys]`.
#[derive(Debug, Serialize)]
pub(super) struct ReviewKeys {
    reveal: Vec<KeyDto>,
    failed: Vec<KeyDto>,
    partly: Vec<KeyDto>,
    passed: Vec<KeyDto>,
    up: Vec<KeyDto>,
    down: Vec<KeyDto>,
    skip: Vec<KeyDto>,
    remove: Vec<KeyDto>,
    restart: Vec<KeyDto>,
    ask: Vec<KeyDto>,
    make_note: Vec<KeyDto>,
    make_card: Vec<KeyDto>,
}

impl ReviewKeys {
    pub(super) fn from(b: &Bindings) -> Self {
        Self {
            reveal: key_list(&b.reveal),
            failed: key_list(&b.failed),
            partly: key_list(&b.partly),
            passed: key_list(&b.passed),
            up: key_list(&b.up),
            down: key_list(&b.down),
            skip: key_list(&b.skip),
            remove: key_list(&b.remove),
            restart: key_list(&b.restart),
            ask: key_list(&b.ask),
            make_note: key_list(&b.make_note),
            make_card: key_list(&b.make_card),
        }
    }
}

/// The deck-picker navigation keys the selection screen binds, mirroring the
/// configured `[picker]` section (Vim defaults): move, open/back, focus the
/// filter, open the Mastered window, and the depth menu (open it + start at
/// one of the three depths). (Jump to first/last is fixed at `g`/`G`/Home/End
/// on the page, so it isn't sent.)
#[derive(Debug, Serialize)]
pub(super) struct PickerKeysDto {
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
    pub(super) fn from(k: &PickerKeys) -> Self {
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
pub(super) struct BrowseKeys {
    next: Vec<KeyDto>,
    prev: Vec<KeyDto>,
    remove: Vec<KeyDto>,
}

impl BrowseKeys {
    pub(super) fn from(b: &BrowseBindings) -> Self {
        Self {
            next: key_list(&b.next),
            prev: key_list(&b.prev),
            remove: key_list(&b.remove),
        }
    }
}

/// The exam payload sent to the browser. The page renders sub-views off `phase`
/// and polls `GET /api/exam` while `thinking`.
#[derive(Serialize)]
pub(super) struct ExamDto {
    pub(super) phase: &'static str,
    pub(super) deck: String,
    pub(super) strictness: &'static str,
    pub(super) total: usize,
    pub(super) current: usize,
    /// The current question's prompt (in the answering phase).
    pub(super) question: Option<String>,
    /// The answer saved for the current question so far.
    pub(super) answer: String,
    pub(super) on_last: bool,
    /// Per-question breakdown (results phase).
    pub(super) grades: Vec<ExamGradeDto>,
    pub(super) passed: Option<bool>,
    pub(super) gaps: Vec<String>,
    /// Whether a failed result can be remediated into cards (fact decks only — a
    /// trace is re-walked, not remediated). Drives the remediation button.
    pub(super) can_remediate: bool,
    /// How many remediation cards the last remediation created or revived;
    /// `None` until one completes (the "remediated" phase renders it).
    pub(super) remediated_count: Option<usize>,
    /// A trace exam (the compression), so the page shows a "re-walk" hint on a
    /// fail instead of remediation.
    pub(super) is_trace: bool,
    /// Decks a pass unlocks.
    pub(super) unlocks: Vec<String>,
    pub(super) thinking: bool,
    pub(super) error: Option<String>,
    /// Seconds the in-flight Claude call has been running (progress feedback).
    pub(super) elapsed: Option<u64>,
    /// Milliseconds until a failed trace exam may be re-sat — set only in the
    /// `cooldown` phase, `null` everywhere else.
    pub(super) cooldown_ms: Option<u64>,
}

/// The `phase:"cooldown"` ExamDto for a trace exam still cooling down after a
/// fail: no sitting exists, so every session field is at its baseline and only
/// `deck` + `cooldown_ms` carry information.
pub(super) fn cooldown_dto(deck: &str, cooldown_ms: u64) -> ExamDto {
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
pub(super) struct ExamGradeDto {
    pub(super) question: String,
    pub(super) points: Vec<String>,
    pub(super) answer: String,
    pub(super) verdict: &'static str,
    pub(super) feedback: String,
    pub(super) missed: Vec<String>,
}

pub(super) fn exam_phase_name(phase: &exam::Phase) -> &'static str {
    match phase {
        exam::Phase::Generating => "generating",
        exam::Phase::Answering => "answering",
        exam::Phase::Grading => "grading",
        exam::Phase::Results => "results",
        exam::Phase::Remediating => "remediating",
        exam::Phase::Remediated => "remediated",
    }
}

pub(super) fn strictness_name(s: Strictness) -> &'static str {
    match s {
        Strictness::Strict => "strict",
        Strictness::Balanced => "balanced",
        Strictness::Lenient => "lenient",
    }
}

/// Serializes an exam sitting for the browser; `decks_dir` resolves the
/// dependent decks shown on a pass.
pub(super) fn exam_dto(ex: &Examining, decks_dir: &Path) -> ExamDto {
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

/// The card context a paired phone sends with a remote tutor call
/// (`/api/remote/*`): the server holds no session for remote clients, so
/// every request carries the card in full.
#[derive(Debug, Deserialize)]
pub(super) struct RemoteCard {
    pub(super) subject: String,
    pub(super) front: String,
    pub(super) back: Vec<String>,
    /// The card's `% at:` source citation, if any.
    pub(super) at: Option<String>,
}

/// One prior tutor exchange, re-sent by the phone with every call: the
/// phone owns the transcript, the server never stores or replays one.
#[derive(Debug, Deserialize)]
pub(super) struct RemoteTurn {
    pub(super) q: String,
    pub(super) a: String,
}

/// `POST /api/remote/ask`: a paired phone's tutor question, with the card and
/// the prior exchanges it needs re-sent alongside it.
#[derive(Debug, Deserialize)]
pub(super) struct RemoteAskReq {
    pub(super) card: RemoteCard,
    pub(super) history: Vec<RemoteTurn>,
    pub(super) question: String,
}

/// `POST /api/remote/ask/draft`: asks the tutor to draft a card from the
/// exchange so far.
#[derive(Debug, Deserialize)]
pub(super) struct RemoteDraftReq {
    pub(super) card: RemoteCard,
    pub(super) history: Vec<RemoteTurn>,
}

/// `POST /api/remote/ask/note`: asks the tutor to condense the exchange so
/// far into note lines. Same shape as [`RemoteDraftReq`] (card + history, no
/// extra field); kept as its own type so each remote request's name matches
/// its own endpoint, like [`RemoteAskReq`]/[`RemoteDraftReq`].
#[derive(Debug, Deserialize)]
pub(super) struct RemoteNoteReq {
    pub(super) card: RemoteCard,
    pub(super) history: Vec<RemoteTurn>,
}

/// The reply to a remote tutor call. The phone polls while `thinking`, like
/// the browser's [`AskDto`], but carries no transcript of its own: the
/// phone already holds it, so this is just the newest turn's outcome.
#[derive(Debug, Serialize)]
pub(super) struct RemoteAskDto {
    pub(super) thinking: bool,
    pub(super) answer: Option<String>,
    pub(super) draft: Option<DraftCardDto>,
    /// Condensed note lines (at most three) from a note call. `Some` only
    /// for a settled note outcome; an empty vec is itself a valid settled
    /// result ("nothing to save"), not an error.
    pub(super) note: Option<Vec<String>>,
    pub(super) error: Option<String>,
    /// Seconds the in-flight backend call has been running, like
    /// [`ExamDto::elapsed`].
    pub(super) elapsed: Option<u64>,
}

/// The AI exam over `/api/remote/*`: a paired phone sits an exam with no
/// server-side session either. Unlike [`ExamDto`], answering happens
/// phone-local and is graded as one batch, so there is no
/// `total`/`current`/`question`/`answer`/`on_last`; the phone counts its own
/// remediation cards, so there is no `remediated_count`; and there is no
/// server-side cooldown or `unlocks` (the server's store is not the phone's
/// truth, so a trace re-sit is never gated here, and a pass unlocks nothing
/// server-side, so the phone applies both to its own state). A trace deck now
/// starts and sits like a fact deck, distinguished by `is_trace`.
#[derive(Serialize)]
pub(super) struct RemoteExamDto {
    /// `idle | generating | answering | grading | results | remediating |
    /// remediated` (open set, like [`ExamDto::phase`]).
    pub(super) phase: &'static str,
    pub(super) deck: String,
    pub(super) strictness: &'static str,
    /// Prompts only; the rubric never leaves the server.
    pub(super) questions: Vec<String>,
    pub(super) passed: Option<bool>,
    pub(super) grades: Vec<ExamGradeDto>,
    pub(super) gaps: Vec<String>,
    /// Always false for a trace sitting (a failed compression is re-walked,
    /// not remediated), like [`ExamDto::can_remediate`].
    pub(super) can_remediate: bool,
    /// Deck-format text, set in the `remediated` phase. The phone stores
    /// these cards; the server never does.
    pub(super) cards: Option<String>,
    /// A trace (compression) sitting vs a fact-deck sitting, like
    /// [`ExamDto::is_trace`]. `false` at `idle`.
    pub(super) is_trace: bool,
    pub(super) thinking: bool,
    pub(super) elapsed: Option<u64>,
    pub(super) error: Option<String>,
}

/// `POST`/`GET /api/remote/generate`: mirrors [`GenerateDto`], but the
/// server never places the result (a paired phone owns its own destination
/// and collision handling), so this carries the full deck text back instead
/// of a saved file name, plus a suggested one for the phone to use.
#[derive(Serialize)]
pub(super) struct RemoteGenerateDto {
    /// `generating` | `done` | `error` (open set).
    pub(super) phase: &'static str,
    /// The full generated deck text, set only in `done`.
    pub(super) deck: Option<String>,
    /// A suggested file name (`generate::deck_name`), set only in `done`;
    /// the phone decides where and under what name to save it.
    pub(super) filename: Option<String>,
    /// The finished text's parsed card count, best-effort: `null` if it does
    /// not parse. Unlike [`GenerateDto`], a parse failure here does not flip
    /// `phase` to `error`, since the server never saves the file either way:
    /// there is nothing to warn "saved but broken" about; the phone parses
    /// and validates its own copy.
    pub(super) cards: Option<usize>,
    pub(super) elapsed: Option<u64>,
    pub(super) error: Option<String>,
}

/// The Augment screen payload. `rows` is a flat, data-driven list so a new
/// augmentation target is one more row with no page-layout change.
#[derive(Serialize)]
pub(super) struct AugmentDto {
    pub(super) deck: String,
    pub(super) cards: usize,
    pub(super) rows: Vec<AugmentRowDto>,
    /// The target currently generating, if any (the page shows a spinner + polls).
    pub(super) busy: Option<&'static str>,
    /// Seconds the in-flight generation has run (progress feedback).
    pub(super) elapsed: Option<u64>,
    pub(super) error: Option<String>,
    /// Targets still waiting behind the busy one in the current batch.
    pub(super) queued: Vec<&'static str>,
    /// Targets the current batch has already finished successfully.
    pub(super) done: Vec<&'static str>,
    /// Targets the current batch attempted and failed (partial-failure safe:
    /// one target's error doesn't stop the rest from running).
    pub(super) failed: Vec<FailedTargetDto>,
}

/// One batch target that failed, with the error the worker returned.
#[derive(Debug, Clone, Serialize)]
pub(super) struct FailedTargetDto {
    pub(super) target: &'static str,
    pub(super) error: String,
}

/// One target's row. Per-card targets carry `covered`/`eligible`; the topology
/// row carries its named `items` instead (the page branches on `kind`).
#[derive(Serialize)]
pub(super) struct AugmentRowDto {
    pub(super) kind: &'static str,
    pub(super) label: &'static str,
    pub(super) covered: usize,
    pub(super) eligible: usize,
    pub(super) items: Vec<String>,
    pub(super) busy: bool,
}

#[derive(Serialize)]
pub(super) struct GenerateDto {
    /// `generating` | `done` | `error` (open set).
    pub(super) phase: &'static str,
    pub(super) deck: Option<String>,
    pub(super) cards: Option<usize>,
    pub(super) elapsed: Option<u64>,
    pub(super) error: Option<String>,
}

#[derive(Serialize)]
pub(super) struct ShareDto {
    /// `staging` | `code` | `sent` | `error` (open set).
    pub(super) phase: &'static str,
    pub(super) code: Option<String>,
    pub(super) elapsed: Option<u64>,
    pub(super) error: Option<String>,
}

#[derive(Serialize)]
pub(super) struct ReceiveDto {
    /// `receiving` | `done` | `error` (open set).
    pub(super) phase: &'static str,
    pub(super) landed: Option<String>,
    pub(super) stripped: Vec<String>,
    pub(super) elapsed: Option<u64>,
    pub(super) error: Option<String>,
}

/// One checkpoint as a node on the path rail.
#[derive(Serialize)]
pub(super) struct HopDto {
    pub(super) prompt: String,
    /// `passed` | `partly` | `failed` once judged; `null` while unwalked.
    pub(super) delta: Option<&'static str>,
    /// The hop currently being predicted or revealed.
    pub(super) current: bool,
}

/// A revealed source excerpt for the browser — line-numbered, contiguous.
#[derive(Debug, Serialize)]
pub(super) struct ExcerptDto {
    pub(super) path: String,
    pub(super) lines: Vec<LineDto>,
    pub(super) truncated: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct LineDto {
    pub(super) n: usize,
    pub(super) text: String,
}

/// The walk tally shown on the done screen.
#[derive(Serialize)]
pub(super) struct SummaryDto {
    pub(super) passed: usize,
    pub(super) partly: usize,
    pub(super) failed: usize,
    /// 1-based hop numbers judged partly or failed.
    pub(super) weak: Vec<usize>,
    pub(super) total: usize,
}

/// The trace-walk payload sent to the browser. The page renders sub-views off
/// `phase` and polls `GET /api/walk` while `thinking` (a live grade in flight).
#[derive(Serialize)]
pub(super) struct WalkDto {
    /// Discriminates this trace-walk payload from the review [`StateDto`] for the
    /// single client dispatcher (`isWalk`): always `"walk"`.
    pub(super) kind: &'static str,
    pub(super) phase: &'static str,
    pub(super) description: String,
    pub(super) source: Option<String>,
    pub(super) total: usize,
    /// 1-based index of the hop being walked.
    pub(super) current: usize,
    /// The path rail — one node per checkpoint.
    pub(super) path: Vec<HopDto>,
    // predict + reveal
    pub(super) prompt: Option<String>,
    pub(super) givens: Vec<String>,
    pub(super) locator: Option<String>,
    /// What the learner predicted (shown on reveal).
    pub(super) prediction: Option<String>,
    // reveal
    pub(super) excerpt: Option<ExcerptDto>,
    pub(super) excerpt_error: Option<String>,
    pub(super) points: Vec<String>,
    pub(super) note: Option<String>,
    /// `--grade` mode: Claude judges instead of the learner.
    pub(super) auto_grade: bool,
    /// A live grade is in flight.
    pub(super) thinking: bool,
    pub(super) verdict: Option<&'static str>,
    pub(super) feedback: Option<String>,
    /// A live grade failed — the reveal offers self-grading instead.
    pub(super) grade_error: Option<String>,
    // done
    pub(super) summary: Option<SummaryDto>,
}

pub(super) fn walk_phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Predict => "predict",
        Phase::Reveal => "reveal",
        Phase::Done => "done",
    }
}

pub(super) fn delta_name(delta: Delta) -> &'static str {
    match delta {
        Delta::Passed => "passed",
        Delta::Partial => "partly",
        Delta::Failed => "failed",
    }
}

pub(super) fn excerpt_dto(excerpt: &Excerpt) -> ExcerptDto {
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
pub(super) fn walk_dto(w: &Walking) -> WalkDto {
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

/// Serializes the current browse phase for the page: the cards in browse phase,
/// or an empty list flagged `phase: "select"` for the deck-selection screen.
pub(super) fn browse_payload(browsing: Option<&Browsing>) -> BrowseDto {
    match browsing {
        Some(b) => BrowseDto {
            phase: "browse",
            label: b.label.clone(),
            cards: b.cards.iter().map(|c| card_dto(c.into())).collect(),
        },
        None => BrowseDto {
            phase: "select",
            label: "select decks".to_string(),
            cards: Vec::new(),
        },
    }
}

/// Builds the state payload. In the select phase (`reviewing` is `None`) it
/// reports `phase: "select"` with no card; otherwise it serializes the live
/// session and store. For a choice card it also builds the options via
/// [`current_question`], seeded by the card id plus its appearance count so
/// they are stable across the `/api/state` and `/api/choose` requests without
/// any server-side caching, yet reshuffle the next time the card is served.
pub(super) fn review_state(reviewing: Option<&Reviewing>, store: &Store) -> StateDto {
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
            acquired: 0,
            exam_due: Vec::new(),
            can_restart: false,
            promotable: false,
            label: "select decks".to_string(),
        };
    };
    let session = &r.session;
    // ONE core build: every session/store-derived review fact comes from the
    // shared contract (`crate::review`), the same state the embedded mobile
    // client renders. This envelope only adds the wire naming, the phase, and
    // serve-held context (the label, the exam-due deck scan, and the card
    // enrichment below), never a re-derivation of a fact the core state
    // already carries.
    let s = review::state(session, store, &r.augment, None);
    // On a finished session, surface any deck that just reached "exam due" so the
    // summary can offer to sit it. Only computed when finished (it reloads decks).
    let exam_due = if s.finished {
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
    // The core view is the card's substance; this closure only adds what needs
    // the serve-held maps (crumb, citation) on top.
    let card_with_citation = s.card.zip(session.current()).map(|(view, c)| {
        let mut dto = card_dto(view);
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
            // `dto.at` already carries the raw locator via the core view; a
            // resolved excerpt may relabel it to its display form below.
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
        phase: if s.finished { "done" } else { "review" },
        card: card_with_citation,
        choices: s.choices,
        keypoints: s.keypoints,
        acquire: s.acquire,
        mode: mode_name(s.mode),
        depth: depth_name(s.depth),
        input: input_name(s.input),
        remaining: s.remaining,
        initial: s.initial,
        reviews: s.reviews,
        passed: s.passed,
        failed: s.failed,
        acquired: s.acquired,
        exam_due,
        can_restart: s.can_restart,
        promotable: s.promotable,
        label: r.label.clone(),
    }
}

/// A `DeckState` as the machine-readable string the page styles rows by.
pub(super) fn state_name(s: DeckState) -> &'static str {
    match s {
        DeckState::NotStarted => "new",
        DeckState::Started => "started",
        DeckState::Finished => "finished",
        DeckState::ExamDue => "examdue",
    }
}

/// Builds the focus-drawer payload for a `deck`: its own stored topologies (the
/// cache can be shared by several decks on one store, so they're scoped by card
/// membership), each region's per-card strength heatmap and due/new count, and
/// the whole-deck due count — all read against the deck's `store`.
pub(super) fn deck_topology_dto(
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
    let scheduler = Fsrs::new(review.retention, review.acquire_cooldown_ms);
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

/// Serializes a card for the browser from its core [`CardView`]. The only
/// transport touch is mapping the view's image paths to `/img/<key>` URLs
/// (the same hash the registry derives from the path). The citation and
/// crumb are resolved by `review_state`, which holds the source base and
/// topology; browse leaves them empty.
pub(super) fn card_dto(view: CardView) -> CardDto {
    let img_url = |p: &str| format!("/img/{}", img_key(Path::new(p)));
    CardDto {
        img: view.image.as_deref().map(img_url),
        img_back: view.image_back.as_deref().map(img_url),
        front: view.front,
        context: view.context,
        back: view.back,
        reshaped: view.reshaped,
        note: view.note,
        at: view.at,
        citation: None,
        citation_error: None,
        crumb: None,
    }
}

/// The CLI/value name of an input method, matching `Input`'s clap names.
pub(super) fn input_name(input: Input) -> &'static str {
    match input {
        Input::Type => "type",
        Input::Draw => "draw",
    }
}
