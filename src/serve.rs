//! A local web frontend.
//!
//! `flash serve` starts a small synchronous HTTP server (one request at a
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
    collections::{BTreeSet, HashMap},
    hash::Hasher,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, TryRecvError},
};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server};
use twox_hash::XxHash64;

use crate::{
    answer::{Mode, grade_lines_unordered},
    ask::{self, CliSession, Exchange, Reply},
    card::Card,
    choice,
    config::{
        AskConfig, Bindings, BrowseBindings, ExamConfig, Key, KeyPattern, PickerKeys, Strictness,
    },
    deck::{self, Deck, DeckState},
    exam, picker,
    recent::RecentDecks,
    render::{self, NoteUnit},
    scheduler::{Grade, SchedulerKind},
    session::{Session, now_ms},
    store::Store,
    trace::{self, Delta, Excerpt, Phase, Walk},
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
const BROWSE_HTML: &str = include_str!("../assets/serve/browse.html");
const WALK_HTML: &str = include_str!("../assets/serve/walk.html");

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
    note: Vec<NoteUnitDto>,
    /// `/img/<key>` URL for the question-side image, or `null`.
    img: Option<String>,
    /// `/img/<key>` URL for the answer-side image, shown on reveal, or `null`.
    img_back: Option<String>,
}

/// The current review state sent to the browser after every action.
#[derive(Debug, Serialize)]
struct StateDto {
    /// `"select"` while choosing decks (no session yet), else `"review"`.
    phase: &'static str,
    /// The card up for review, or `null` when finished or in the select phase.
    card: Option<CardDto>,
    /// For `choice` mode, the multiple-choice options (one is correct); `null`
    /// otherwise, or when the card has too few distractors (the page then
    /// falls back to reveal). The correct index is never sent here.
    choices: Option<Vec<String>>,
    /// The answer mode name (`flip`, `line`, …); the page reveals
    /// line-by-line for `line` and flip-style otherwise.
    mode: &'static str,
    remaining: usize,
    initial: usize,
    reviews: usize,
    passed: usize,
    failed: usize,
    /// Per-stage counts (index 0 = unseen).
    histogram: [usize; 6],
    finished: bool,
    /// Subjects of decks in this (finished) session that are now `ExamDue` —
    /// drilled, sourced, and not yet mastered. The summary offers to sit each.
    /// Empty until the session is finished.
    exam_due: Vec<String>,
    /// Whether a restart would find any due/new cards right now. The summary
    /// disables "New session" and shows a "nothing due" note when this is
    /// false.
    can_restart: bool,
    /// The highest reachable stage (1–5); the page renders stages above it as a
    /// muted `–` since every card caps below them via `% max-stage:`.
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
    /// `true` when this entry has recent-use history (shown in Recent by
    /// default; the rest are reachable through the filter).
    recent: bool,
    /// `true` for a workspace/folder row (opens into its members on click).
    is_workspace: bool,
    /// For a workspace/folder row: its member decks as an unlock dependency
    /// tree, shown when you open it.
    members: Vec<MemberDto>,
    /// Dim location hint (parent dir) for entries outside the decks dir; `null`
    /// keeps the row clean. Disambiguates same-named decks/workspaces.
    path: Option<String>,
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
    depth: usize,
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
    again: Vec<KeyDto>,
    good: Vec<KeyDto>,
    easy: Vec<KeyDto>,
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
            again: key_list(&b.again),
            good: key_list(&b.good),
            easy: key_list(&b.easy),
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
    /// CLI `--mode` override; `None` lets each card use its own mode.
    pub mode_override: Option<Mode>,
    pub keys: Bindings,
    /// Deck-picker navigation keys (the `[picker]` section), bound on the
    /// selection screen.
    pub picker: PickerKeys,
    /// Fuzzy-mode typo tolerance per line.
    pub max_typos: usize,
    /// Ask-Claude settings (command, allowlist, timeout, …).
    pub ask: AskConfig,
    /// AI-exam settings (model, question count, default strictness, …).
    pub exam: ExamConfig,
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
}

/// A trace walk ready to serve, built when a single trace deck is picked from the
/// review server's deck-selection screen. The walk is self-graded (no live
/// `--grade`), matching the terminal picker's trace → walk.
pub struct WalkBuild {
    pub walk: Walk,
    pub scheduler: SchedulerKind,
}

/// The `/api/select` reply when a trace was picked: tells the page to navigate to
/// the walk view (the review server hosts `walk.html` at `/walk`).
#[derive(Debug, Serialize)]
struct WalkRedirect {
    redirect: &'static str,
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
    /// Ask-Claude: the CLI conversation spanning this selection, the running
    /// transcript, the per-subject `% link:` links, and an in-flight CLI call.
    cli: CliSession,
    transcript: Vec<Exchange>,
    links: HashMap<String, Vec<String>>,
    pending: Option<Pending>,
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

impl Reviewing {
    fn new(build: SessionBuild) -> Self {
        let images = collect_images(build.session.cards());
        Self {
            session: build.session,
            label: build.label,
            files: DeckFiles::new(build.decks),
            images,
            cli: CliSession::new(),
            transcript: Vec::new(),
            links: build.links,
            pending: None,
        }
    }

    /// The ask-view payload, with an optional one-shot status/error.
    fn ask_dto(&self, status: Option<String>, error: Option<String>) -> AskDto {
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

    /// Starts an ask-Claude call about the current card. `question` is the text
    /// to ask; `None` condenses the conversation into deck notes instead.
    /// Returns `false` (no-op) if a call is already pending, nothing is
    /// reviewable, or there is nothing to condense.
    fn start_ask(&mut self, cfg: &AskConfig, question: Option<String>) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let Some(card) = self.session.current().cloned() else {
            return false;
        };
        let (prompt, purpose) = match question {
            Some(q) => {
                let links = self.links.get(&*card.subject).cloned().unwrap_or_default();
                let prompt = ask::question_prompt(&card, &links, &q, !self.cli.started);
                (prompt, Purpose::Question(q))
            }
            None => {
                if self.transcript.is_empty() {
                    return false;
                }
                (
                    ask::condense_prompt(&card, &self.transcript),
                    Purpose::Condense,
                )
            }
        };
        let rx = ask::spawn(cfg.clone(), prompt, self.cli.args());
        self.pending = Some(Pending { rx, purpose, card });
        true
    }

    /// Drains a finished CLI reply into the transcript (a question) or the deck
    /// file (a condense). Returns a transient `(status, error)` to show once.
    fn poll_ask(&mut self) -> (Option<String>, Option<String>) {
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
                match self
                    .files
                    .append_note(&pending.card.subject, pending.card.line, &notes)
                {
                    Ok(()) => (Some("note saved".to_string()), None),
                    Err(e) => (None, Some(e)),
                }
            }
            // Don't resume a session in an unknown state; the next question
            // starts a fresh one.
            (Reply::Error(e), _) => {
                self.cli = CliSession::new();
                (None, Some(e))
            }
        }
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
    /// Decks a pass unlocks.
    unlocks: Vec<String>,
    thinking: bool,
    error: Option<String>,
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
        unlocks,
        thinking: s.thinking(),
        error: s.error().map(str::to_string),
    }
}

/// Serves review at `addr` until the process is stopped. When `initial` is
/// `None` the server opens at the in-browser deck-selection screen; picking
/// decks (`POST /api/select`) calls `build` to construct a session in place.
/// `build` borrows the shared `store` and `recent`, so all sessions write one
/// history and update the recent-decks list, exactly like the CLI.
#[allow(clippy::too_many_arguments)] // each is a distinct, named server input
pub fn run_review(
    initial: Option<SessionBuild>,
    mut store: Store,
    mut recent: RecentDecks,
    decks_dir: PathBuf,
    addr: SocketAddr,
    opts: ReviewOptions,
    mut build: impl FnMut(Vec<PathBuf>, &Store, &mut RecentDecks) -> Result<SessionBuild>,
    // Builds a walk when the picked decks are a single trace (else `None`, so the
    // caller flattens to a review); mirrors the terminal picker's trace → walk.
    mut build_walk: impl FnMut(&[PathBuf]) -> Result<Option<WalkBuild>>,
) -> Result<()> {
    let ReviewOptions {
        mode_override,
        keys: bindings,
        picker: picker_keys,
        max_typos,
        ask: ask_cfg,
        exam: exam_cfg,
    } = opts;
    let keys = ReviewKeys::from(&bindings);
    let picker_keys = PickerKeysDto::from(&picker_keys);
    let mut reviewing = initial.map(Reviewing::new);
    let mut examining: Option<Examining> = None;
    // A trace picked from the selection screen walks here (the page navigates to
    // `/walk`, which this server hosts alongside review).
    let mut walking: Option<Walking> = None;
    let server = Server::http(addr).map_err(|e| anyhow!("cannot start server on {addr}: {e}"))?;
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let path = request_path(&request);
        match (&method, path.as_str()) {
            (Method::Get, "/") => respond_html(request, REVIEW_HTML),
            (Method::Get, "/walk") => respond_html(request, WALK_HTML),
            (Method::Get, "/api/keys") => respond_json(request, &keys),
            (Method::Get, "/api/picker-keys") => respond_json(request, &picker_keys),
            (Method::Get, "/api/decks") => {
                // Review enforces locking; the picker won't start a locked deck.
                respond_json(request, &deck_catalog(&decks_dir, &recent, &store, true))
            }
            (Method::Get, key) if key.starts_with("/img/") => match &reviewing {
                Some(r) => serve_image(request, &r.images, &key["/img/".len()..]),
                None => respond_status(request, 404),
            },
            (Method::Get, "/api/state") => respond_json(
                request,
                &review_state(reviewing.as_ref(), &store, mode_override),
            ),
            (Method::Post, "/api/select") => {
                match select_decks(&mut request, &decks_dir, &recent) {
                    // A single trace deck walks (the page navigates to `/walk`)
                    // rather than flattening into a card review.
                    Some(paths) => match build_walk(&paths) {
                        Ok(Some(wb)) => {
                            walking = Some(Walking::new(wb.walk, wb.scheduler, None));
                            reviewing = None;
                            examining = None;
                            respond_json(request, &WalkRedirect { redirect: "/walk" });
                        }
                        Ok(None) => match build(paths, &store, &mut recent) {
                            Ok(b) => {
                                reviewing = Some(Reviewing::new(b));
                                walking = None;
                                respond_json(
                                    request,
                                    &review_state(reviewing.as_ref(), &store, mode_override),
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
                    },
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/deselect") => {
                reviewing = None;
                walking = None;
                respond_json(
                    request,
                    &review_state(reviewing.as_ref(), &store, mode_override),
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
                        respond_json(
                            request,
                            &review_state(reviewing.as_ref(), &store, mode_override),
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
                r.session.skip();
                respond_json(
                    request,
                    &review_state(reviewing.as_ref(), &store, mode_override),
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
                    let mode = mode_override.or(card.mode).unwrap_or_default();
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
                    let correct = choice::build(&card, r.session.cards(), card.id())?.correct;
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
                let dropped = r.session.remove_current();
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
                    &review_state(reviewing.as_ref(), &store, mode_override),
                );
            }
            (Method::Post, "/api/restart") => {
                let Some(r) = reviewing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                r.session.restart(&store, now_ms());
                respond_json(
                    request,
                    &review_state(reviewing.as_ref(), &store, mode_override),
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
                let known: HashMap<String, PathBuf> = picker::catalog(&decks_dir, &recent)
                    .into_iter()
                    .map(|e| (e.name, e.path))
                    .collect();
                let Some(path) = body.and_then(|b| known.get(&b.deck).cloned()) else {
                    respond_status(request, 400);
                    continue;
                };
                match Deck::load(&path) {
                    Ok(deck)
                        if !deck.sources.is_empty()
                            && matches!(
                                deck.state(&store),
                                DeckState::ExamDue | DeckState::Finished
                            ) =>
                    {
                        let strictness =
                            deck.settings.exam_strictness.unwrap_or(exam_cfg.strictness);
                        let sitting = exam::Sitting::start(
                            &deck,
                            strictness,
                            exam_cfg.clone(),
                            ask_cfg.clone(),
                        );
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
                ex.sitting.poll(&mut store, now_ms());
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
            // On a fail, generate + append remediation cards to the deck.
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
                respond_json(
                    request,
                    &review_state(reviewing.as_ref(), &store, mode_override),
                );
            }
            // ── Trace walk (a single trace picked from the selection screen) ──
            // The same flow as the standalone `run_walk`, guarded on `walking`.
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
            (Method::Post, "/api/walk/compress") => {
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
                    w.walk.compress(b.text);
                }
                respond_json(request, &walk_dto(w));
            }
            (Method::Post, "/api/walk/restart") => {
                let Some(w) = walking.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                let fresh = Walk::new(w.walk.trace().clone(), w.scheduler);
                let scheduler = w.scheduler;
                let grade = w.grade.take();
                *w = Walking::new(fresh, scheduler, grade);
                respond_json(request, &walk_dto(w));
            }
            // Back to decks: abandon the walk and return to the picker.
            (Method::Post, "/api/walk/leave") => {
                walking = None;
                respond_status(request, 204);
            }
            _ => respond_status(request, 404),
        }
    }
    Ok(())
}

// ── Trace walks (`flash trace --serve`) ──────────────────────────────────────
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
    scheduler: SchedulerKind,
    /// `Some` in `--grade` mode: the `[ask]` config a background grade uses
    /// (grading runs at the tutor tier, not trace's heavy build defaults).
    grade: Option<AskConfig>,
    /// A background Claude grade in flight for the current reveal.
    pending: Option<Receiver<Result<(Delta, String), String>>>,
    /// The resolved Claude grade for the current reveal (verdict + feedback).
    grade_result: Option<(Delta, String)>,
    /// A failed Claude grade — the reveal falls back to self-grading.
    grade_error: Option<String>,
}

impl Walking {
    fn new(walk: Walk, scheduler: SchedulerKind, grade: Option<AskConfig>) -> Self {
        Walking {
            walk,
            scheduler,
            grade,
            pending: None,
            grade_result: None,
            grade_error: None,
        }
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
    /// `got` | `partial` | `missed` once judged; `null` while unwalked.
    delta: Option<&'static str>,
    /// The hop currently being predicted or revealed.
    current: bool,
}

/// A revealed source excerpt for the browser — line-numbered, contiguous.
#[derive(Serialize)]
struct ExcerptDto {
    path: String,
    lines: Vec<LineDto>,
    truncated: bool,
}

#[derive(Serialize)]
struct LineDto {
    n: usize,
    text: String,
}

/// The walk tally shown on the done screen.
#[derive(Serialize)]
struct SummaryDto {
    got: usize,
    partial: usize,
    missed: usize,
    /// 1-based hop numbers judged Partial or Missed.
    weak: Vec<usize>,
    total: usize,
}

/// The trace-walk payload sent to the browser. The page renders sub-views off
/// `phase` and polls `GET /api/walk` while `thinking` (a live grade in flight).
#[derive(Serialize)]
struct WalkDto {
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
    compression: Option<String>,
}

fn walk_phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Predict => "predict",
        Phase::Reveal => "reveal",
        Phase::Compress => "compress",
        Phase::Done => "done",
    }
}

fn delta_name(delta: Delta) -> &'static str {
    match delta {
        Delta::Got => "got",
        Delta::Partial => "partial",
        Delta::Missed => "missed",
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
        compression: None,
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
                    Ok(ex) => dto.excerpt = Some(excerpt_dto(&ex)),
                    Err(e) => dto.excerpt_error = Some(format!("{e:#}")),
                }
            }
            dto.prediction = walk
                .prediction(walk.current_index())
                .map(str::to_string)
                .filter(|p| !p.is_empty());
        }
        Phase::Compress => {}
        Phase::Done => {
            let s = walk.summary();
            dto.summary = Some(SummaryDto {
                got: s.got,
                partial: s.partial,
                missed: s.missed,
                weak: s.weak.iter().map(|i| i + 1).collect(),
                total: walk.total(),
            });
            dto.compression = walk.compression().map(str::to_string);
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

/// The walk actions the web page binds, reusing the configured review `[keys]`:
/// the three grades map onto again/good/easy (Missed→again, Partial→good,
/// Got it→easy) so the walk feels like review, and `reveal` advances.
#[derive(Debug, Serialize)]
struct WalkKeys {
    reveal: Vec<KeyDto>,
    again: Vec<KeyDto>,
    good: Vec<KeyDto>,
    easy: Vec<KeyDto>,
}

impl WalkKeys {
    fn from(b: &Bindings) -> Self {
        Self {
            reveal: key_list(&b.reveal),
            again: key_list(&b.again),
            good: key_list(&b.good),
            easy: key_list(&b.easy),
        }
    }
}

/// Serves a single trace walk at `addr` until the process is stopped. Mirrors
/// the terminal walk: predict → reveal a live excerpt → grade (self-judged, or
/// by Claude when `grade` is `Some`) → compress, scheduling each checkpoint in
/// `store`. One deck, one walk — there is no deck-selection screen.
pub fn run_walk(
    walk: Walk,
    mut store: Store,
    addr: SocketAddr,
    scheduler: SchedulerKind,
    grade: Option<AskConfig>,
    bindings: Bindings,
) -> Result<()> {
    let mut walking = Walking::new(walk, scheduler, grade);
    let keys = WalkKeys::from(&bindings);
    let server = Server::http(addr).map_err(|e| anyhow!("cannot start server on {addr}: {e}"))?;
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let path = request_path(&request);
        match (&method, path.as_str()) {
            (Method::Get, "/") => respond_html(request, WALK_HTML),
            (Method::Get, "/api/keys") => respond_json(request, &keys),
            // Poll: drain any finished background grade, return state.
            (Method::Get, "/api/walk") => {
                walking.poll();
                respond_json(request, &walk_dto(&walking));
            }
            // Commit the prediction and move to the reveal (spawning a live grade
            // in `--grade` mode).
            (Method::Post, "/api/walk/predict") => {
                #[derive(Deserialize)]
                struct Body {
                    text: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                if let Some(b) = body {
                    walking.walk.predict(b.text);
                    walking.start_grade();
                }
                respond_json(request, &walk_dto(&walking));
            }
            // Record the delta (Claude's if present, else the self-grade in the
            // body), schedule the checkpoint, and advance.
            (Method::Post, "/api/walk/grade") => {
                let self_delta = read_delta(&mut request);
                let delta = walking
                    .grade_result
                    .as_ref()
                    .map(|(d, _)| *d)
                    .or(self_delta);
                match delta {
                    Some(delta) => {
                        walking.walk.grade(&mut store, delta, now_ms());
                        if let Err(e) = store.save() {
                            eprintln!("warning: could not save progress: {e}");
                        }
                        walking.clear_grade();
                        respond_json(request, &walk_dto(&walking));
                    }
                    None => respond_status(request, 400),
                }
            }
            // Record the final compression and finish the walk.
            (Method::Post, "/api/walk/compress") => {
                #[derive(Deserialize)]
                struct Body {
                    text: String,
                }
                let body: Option<Body> = serde_json::from_reader(request.as_reader()).ok();
                if let Some(b) = body {
                    walking.walk.compress(b.text);
                }
                respond_json(request, &walk_dto(&walking));
            }
            // Walk the same trace again from the top.
            (Method::Post, "/api/walk/restart") => {
                let fresh = Walk::new(walking.walk.trace().clone(), walking.scheduler);
                let grade = walking.grade.take();
                walking = Walking::new(fresh, walking.scheduler, grade);
                respond_json(request, &walk_dto(&walking));
            }
            _ => respond_status(request, 404),
        }
    }
    Ok(())
}

/// The server's live browse state once decks are chosen. Its absence (`None`)
/// means the deck-selection phase.
struct Browsing {
    cards: Vec<Card>,
    label: String,
    files: DeckFiles,
    images: HashMap<String, PathBuf>,
}

impl Browsing {
    fn new(build: CardsBuild) -> Self {
        let images = collect_images(&build.cards);
        Self {
            cards: build.cards,
            label: build.label,
            files: DeckFiles::new(build.decks),
            images,
        }
    }
}

/// Serves a read-only walk through cards at `addr`. Like [`run_review`], with
/// `initial` `None` it opens at the deck-selection screen; `POST /api/select`
/// builds the card list via `build`. The only thing it writes is card removal
/// (deletes the card from its deck file and prunes its progress in `store`).
#[allow(clippy::too_many_arguments)] // each is a distinct, named server input
pub fn run_browse(
    initial: Option<CardsBuild>,
    mut store: Store,
    mut recent: RecentDecks,
    decks_dir: PathBuf,
    addr: SocketAddr,
    bindings: BrowseBindings,
    picker: PickerKeys,
    mut build: impl FnMut(Vec<PathBuf>, &mut RecentDecks) -> Result<CardsBuild>,
) -> Result<()> {
    let keys = BrowseKeys::from(&bindings);
    let picker_keys = PickerKeysDto::from(&picker);
    let mut browsing = initial.map(Browsing::new);
    let server = Server::http(addr).map_err(|e| anyhow!("cannot start server on {addr}: {e}"))?;
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let path = request_path(&request);
        match (&method, path.as_str()) {
            (Method::Get, "/") => respond_html(request, BROWSE_HTML),
            (Method::Get, "/api/keys") => respond_json(request, &keys),
            (Method::Get, "/api/picker-keys") => respond_json(request, &picker_keys),
            (Method::Get, "/api/decks") => {
                // Browse is read-only — any deck is browsable, so no lock gating.
                respond_json(request, &deck_catalog(&decks_dir, &recent, &store, false))
            }
            (Method::Get, key) if key.starts_with("/img/") => match &browsing {
                Some(b) => serve_image(request, &b.images, &key["/img/".len()..]),
                None => respond_status(request, 404),
            },
            (Method::Get, "/api/cards") => {
                respond_json(request, &browse_payload(browsing.as_ref()))
            }
            (Method::Post, "/api/select") => {
                match select_decks(&mut request, &decks_dir, &recent) {
                    Some(paths) => match build(paths, &mut recent) {
                        Ok(b) => {
                            browsing = Some(Browsing::new(b));
                            respond_json(request, &browse_payload(browsing.as_ref()));
                        }
                        Err(e) => {
                            eprintln!("warning: could not load the selected decks: {e}");
                            respond_status(request, 400);
                        }
                    },
                    None => respond_status(request, 400),
                }
            }
            (Method::Post, "/api/deselect") => {
                browsing = None;
                respond_json(request, &browse_payload(browsing.as_ref()));
            }
            (Method::Post, "/api/remove") => {
                let Some(b) = browsing.as_mut() else {
                    respond_status(request, 409);
                    continue;
                };
                if let Some(index) = read_index(&mut request)
                    && index < b.cards.len()
                {
                    let subject = b.cards[index].subject.to_string();
                    let line = b.cards[index].line;
                    // Drop the card and any cloze siblings, pruning their
                    // progress as they go.
                    b.cards.retain(|card| {
                        let sibling = card.subject.as_ref() == subject && card.line == line;
                        if sibling {
                            store.remove(card.id());
                        }
                        !sibling
                    });
                    let _ = store.save();
                    b.files.remove_block(&subject, line);
                }
                respond_json(request, &browse_payload(browsing.as_ref()));
            }
            _ => respond_status(request, 404),
        }
    }
    Ok(())
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

/// Builds the state payload. In the select phase (`reviewing` is `None`) it
/// reports `phase: "select"` with no card; otherwise it serializes the live
/// session and store. For choice mode it also builds the options, seeded by the
/// card id so they are stable across the `/api/state` and `/api/choose`
/// requests without any server-side caching.
fn review_state(
    reviewing: Option<&Reviewing>,
    store: &Store,
    mode_override: Option<Mode>,
) -> StateDto {
    let Some(r) = reviewing else {
        return StateDto {
            phase: "select",
            card: None,
            choices: None,
            mode: mode_name(Mode::default()),
            remaining: 0,
            initial: 0,
            reviews: 0,
            passed: 0,
            failed: 0,
            histogram: [0; 6],
            finished: false,
            exam_due: Vec::new(),
            can_restart: false,
            top_stage: crate::store::MAX_STAGE,
            label: "select decks".to_string(),
        };
    };
    let session = &r.session;
    let card = session.current();
    // CLI override wins; otherwise the current card's own mode, else default.
    let mode = mode_override
        .or(card.and_then(|c| c.mode))
        .unwrap_or_default();
    let choices = if mode == Mode::Choice {
        card.and_then(|c| choice::build(c, session.cards(), c.id()).map(|q| q.options))
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
    StateDto {
        phase: "review",
        card: card.map(card_dto),
        choices,
        mode: mode_name(mode),
        remaining: session.remaining(),
        initial: session.initial_size,
        reviews: session.stats.reviews,
        passed: session.stats.passed,
        failed: session.stats.failed,
        histogram: session.stage_histogram(store),
        finished,
        exam_due,
        can_restart: session.has_due_now(store, now_ms()),
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
) -> DeckItemDto {
    let recent = e.last_used_ms.is_some();
    match Deck::load(&e.path) {
        Ok(deck) => {
            let s = picker::deck_status(&deck, store, Some(decks_dir), with_lock);
            DeckItemDto {
                name: e.name.clone(),
                label: e.label.clone(),
                meta: Some(s.badge),
                state: state_name(s.state),
                locked: s.locked,
                reviewable: s.reviewable,
                mastered: s.mastered,
                is_trace: s.is_trace,
                recent,
                is_workspace: false,
                members: Vec::new(),
                path: e.path_hint.clone(),
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
            recent,
            is_workspace: false,
            members: Vec::new(),
            path: e.path_hint.clone(),
        },
    }
}

/// A workspace/folder's members as an unlock dependency tree (the drill-in
/// list): each member nests under the `% requires:` that gates it, siblings
/// startable-first, carrying a `depth` for indentation. Badges/locks come from
/// the workspace's own store (a real workspace) or the global store (a plain
/// folder), matching what a session will write.
fn workspace_members(e: &picker::DeckEntry, decks_dir: &Path, with_lock: bool) -> Vec<MemberDto> {
    let store = if crate::workspace::is_workspace(&e.path) {
        Store::open(crate::workspace::store_path(&e.path)).ok()
    } else {
        crate::store::default_store_path().and_then(|p| Store::open(p).ok())
    };
    let paths: Vec<PathBuf> = e.members.iter().map(|m| m.path.clone()).collect();
    let statuses: Vec<Option<picker::DeckStatus>> = paths
        .iter()
        .map(|p| {
            let st = store.as_ref()?;
            Deck::load(p)
                .ok()
                .map(|d| picker::deck_status(&d, st, Some(decks_dir), with_lock))
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
            let blocked = statuses[i]
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
            match &statuses[i] {
                Some(s) => MemberDto {
                    name: m.name.clone(),
                    label: m.label.clone(),
                    meta: Some(s.badge.clone()),
                    state: state_name(s.state),
                    locked: s.locked,
                    reviewable: s.reviewable,
                    mastered: s.mastered,
                    is_trace: s.is_trace,
                    depth,
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
                    depth,
                },
            }
        })
        .collect()
}

/// Builds the deck-selection catalog in the TUI's three sections — workspaces
/// (each with its last-progress time), recent loose decks, and plain folders —
/// each deck's badge/lock from `store`. `with_lock` is false for the browse
/// screen: locking gates *review* only, so any deck is browsable.
fn deck_catalog(
    decks_dir: &Path,
    recent: &RecentDecks,
    store: &Store,
    with_lock: bool,
) -> DeckListDto {
    let mut workspaces = Vec::new();
    let mut recent_decks = Vec::new();
    let mut folders = Vec::new();
    for e in picker::catalog(decks_dir, recent) {
        // A workspace/folder row: its members open on click; it has no state of
        // its own. A folder with a `flash.toml` is a workspace (shown with its
        // last-progress time); without one it's a plain folder.
        if e.is_workspace {
            let is_ws = crate::workspace::is_workspace(&e.path);
            let members = workspace_members(&e, decks_dir, with_lock);
            let meta = if is_ws {
                match picker::workspace_last_progress(&e.path) {
                    Some(when) => format!("{} decks · {when}", members.len()),
                    None => format!("{} decks", members.len()),
                }
            } else {
                format!("{} decks", members.len())
            };
            let dto = DeckItemDto {
                meta: Some(meta),
                state: if is_ws { "workspace" } else { "folder" },
                locked: false,
                reviewable: true,
                mastered: false,
                is_trace: false,
                recent: e.last_used_ms.is_some(),
                is_workspace: true,
                members,
                path: e.path_hint,
                name: e.name,
                label: e.label,
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
        recent_decks.push(deck_item_dto(&e, store, decks_dir, with_lock));
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
fn select_decks(
    request: &mut Request,
    decks_dir: &Path,
    recent: &RecentDecks,
) -> Option<Vec<PathBuf>> {
    #[derive(Deserialize)]
    struct Body {
        decks: Vec<String>,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    if body.decks.is_empty() {
        return None;
    }
    // The resolution map includes top-level decks/workspaces and every
    // workspace's members (by their qualified `<workspace>/<file>` key), so a
    // subset selection from inside a workspace resolves safely too.
    let mut known: HashMap<String, PathBuf> = HashMap::new();
    for e in picker::catalog(decks_dir, recent) {
        for m in &e.members {
            known.insert(m.name.clone(), m.path.clone());
        }
        known.insert(e.name, e.path);
    }
    resolve_names(body.decks, &known)
}

/// Maps each requested deck name to its catalog path. Returns `None` if any
/// name is not in the catalog, so an unknown or crafted name is rejected
/// wholesale rather than reaching the filesystem.
fn resolve_names(names: Vec<String>, known: &HashMap<String, PathBuf>) -> Option<Vec<PathBuf>> {
    names.into_iter().map(|n| known.get(&n).cloned()).collect()
}

/// Serializes a card for the browser, structuring its note via the shared
/// [`render`] model.
fn card_dto(card: &Card) -> CardDto {
    let img_url = |p: &PathBuf| format!("/img/{}", img_key(p));
    CardDto {
        front: card.front.clone(),
        context: card.context.clone(),
        back: card.back.clone(),
        note: render::note_units(card)
            .into_iter()
            .map(NoteUnitDto::from)
            .collect(),
        img: card.image.as_ref().map(img_url),
        img_back: card.image_back.as_ref().map(img_url),
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

/// The CLI/value name of an answer mode, matching `Mode`'s clap names.
fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Flip => "flip",
        Mode::Typing => "typing",
        Mode::Fuzzy => "fuzzy",
        Mode::Choice => "choice",
        Mode::LineByLine => "line",
        Mode::Explain => "explain",
    }
}

/// Parses a `{"grade":"again|good|easy"}` POST body into a [`Grade`].
fn read_grade(request: &mut Request) -> Option<Grade> {
    #[derive(Deserialize)]
    struct Body {
        grade: String,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    match body.grade.as_str() {
        "again" => Some(Grade::Fail),
        "good" => Some(Grade::Pass),
        "easy" => Some(Grade::Easy),
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
    fn resolve_names_rejects_unknown_deck() {
        let mut known = HashMap::new();
        known.insert("a.txt".to_string(), PathBuf::from("/decks/a.txt"));
        known.insert("b.txt".to_string(), PathBuf::from("/decks/b.txt"));
        // All known -> resolves to their catalog paths.
        assert_eq!(
            resolve_names(vec!["b.txt".to_string(), "a.txt".to_string()], &known),
            Some(vec![
                PathBuf::from("/decks/b.txt"),
                PathBuf::from("/decks/a.txt")
            ])
        );
        // One unknown name (e.g. a traversal attempt) rejects the whole request.
        assert_eq!(
            resolve_names(
                vec!["a.txt".to_string(), "../etc/passwd".to_string()],
                &known
            ),
            None
        );
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
        let dto = review_state(None, &store, None);
        assert_eq!(dto.phase, "select");
        assert!(dto.card.is_none());
        assert!(!dto.finished);
    }

    #[test]
    fn grade_names_map_to_grades() {
        // A guard so the JSON contract and the Grade enum stay in sync.
        assert!(matches!(Grade::Fail, Grade::Fail));
        assert_eq!(mode_name(Mode::LineByLine), "line");
        assert_eq!(mode_name(Mode::Flip), "flip");
        assert_eq!(mode_name(Mode::Explain), "explain");
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
            crate::scheduler::SchedulerKind::Leitner,
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
        });
        (reviewing, card, deck)
    }

    #[test]
    fn poll_ask_records_answer_in_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let (mut r, card, _deck) = one_card_reviewing(dir.path());
        let (tx, rx) = std::sync::mpsc::channel();
        r.pending = Some(Pending {
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
        assert!(r.pending.is_none());
        assert_eq!(1, r.transcript.len());
        assert_eq!("why is s1 invalid?", r.transcript[0].0);
        assert_eq!("because ownership moved", r.transcript[0].1);
        assert!(r.cli.started); // later questions --resume
    }

    #[test]
    fn poll_ask_condense_appends_note_to_deck() {
        let dir = tempfile::tempdir().unwrap();
        let (mut r, card, deck) = one_card_reviewing(dir.path());
        r.transcript.push(("q".to_string(), "a".to_string()));
        let (tx, rx) = std::sync::mpsc::channel();
        r.pending = Some(Pending {
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
        r.cli.started = true;
        let (tx, rx) = std::sync::mpsc::channel();
        r.pending = Some(Pending {
            rx,
            purpose: Purpose::Question("q".to_string()),
            card,
        });
        tx.send(Reply::Error("not logged in".to_string())).unwrap();
        let (status, error) = r.poll_ask();
        assert_eq!(Some("not logged in".to_string()), error);
        assert!(status.is_none());
        assert!(r.pending.is_none());
        assert!(!r.cli.started); // a fresh session next time
        assert!(r.transcript.is_empty());
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
        let walk = Walk::new(trace, SchedulerKind::Leitner);
        let mut w = Walking::new(walk, SchedulerKind::Leitner, None);

        // Predict: prompt + givens, no excerpt yet, the first node is current.
        let d = walk_dto(&w);
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
        w.walk.grade(&mut store, Delta::Got, 1000);
        let d = walk_dto(&w);
        assert_eq!("predict", d.phase);
        assert_eq!(2, d.current);
        assert_eq!(Some("got"), d.path[0].delta);
        assert!(d.path[1].current);

        // Walk the last hop, then compress → done with a summary.
        w.walk.predict(String::new());
        w.walk.grade(&mut store, Delta::Missed, 1001);
        assert_eq!("compress", walk_dto(&w).phase);
        w.walk.compress("retraced".to_string());
        let d = walk_dto(&w);
        assert_eq!("done", d.phase);
        let s = d.summary.expect("done has a summary");
        assert_eq!((1, 0, 1), (s.got, s.partial, s.missed));
        assert_eq!(vec![2], s.weak); // 1-based: the missed second hop
        assert_eq!(Some("retraced".to_string()), d.compression);
    }

    #[test]
    fn walk_dto_surfaces_a_live_grade_and_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let trace = walk_deck(dir.path());
        let walk = Walk::new(trace, SchedulerKind::Leitner);
        let mut w = Walking::new(walk, SchedulerKind::Leitner, Some(AskConfig::default()));

        w.walk.predict("g".to_string());
        // Simulate the background grade resolving (no real CLI call in the test).
        w.grade_result = Some((Delta::Partial, "right idea, missed a detail".to_string()));
        let d = walk_dto(&w);
        assert!(d.auto_grade);
        assert_eq!(Some("PARTIAL"), d.verdict);
        assert_eq!(Some("right idea, missed a detail".to_string()), d.feedback);

        w.clear_grade();
        let d = walk_dto(&w);
        assert!(d.verdict.is_none() && d.feedback.is_none() && !d.thinking);
    }
}
