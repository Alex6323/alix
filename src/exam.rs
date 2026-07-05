//! The AI exam — frontend-agnostic engine.
//!
//! A deck's mechanical drill *loads* its material; this exam *verifies
//! understanding* and gates progression. It grades against the deck's declared
//! `% source:` (a URL Claude reads with WebFetch, or a local file embedded in
//! the prompt) — never the cards, which avoids circularity. Three Claude calls,
//! each through the same CLI runner [`crate::ask::run`] that `generate` uses:
//!
//! 1. [`generate_questions`] — fresh open understanding questions from the source, each with the
//!    key points a correct answer must contain.
//! 2. [`grade_answers`] — a strict examiner grades the typed answers against those points and
//!    returns an overall pass/fail by threshold.
//! 3. [`remediation_cards`] — on a fail, turns the missed concepts into cards (cloze/plain for
//!    facts, `% mode: explain` for concepts), as deck-format text ready to append.
//!
//! The engine is pure: it builds prompts, calls the CLI and parses JSON. A CLI
//! consumer (`alix exam`) drives the terminal Q&A; a web exam surface can
//! reuse the same three functions.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, TryRecvError, channel},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::{
    ask,
    config::{AskConfig, ExamConfig, Strictness},
    deck::{self, Deck},
    parser,
    store::{CardState, Store, VirtualCard, VirtualContent, VirtualKind, virtual_id},
};

/// Largest embedded local source file, in bytes. Larger files are truncated
/// (with a marker) so the prompt stays within the model's context.
const MAX_SOURCE_BYTES: usize = 100_000;

/// One open exam question with the key points a correct answer must cover.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ExamQuestion {
    /// The question shown to the student.
    pub prompt: String,
    /// The points a full answer must demonstrate (the grading rubric).
    pub points: Vec<String>,
}

/// How well one answer did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// Covered the key points.
    Pass,
    /// Partially correct — some points missed.
    Partial,
    /// Did not demonstrate understanding.
    Fail,
}

impl Verdict {
    /// The label shown to the student.
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Partial => "PARTIAL",
            Verdict::Fail => "FAIL",
        }
    }
}

/// The grade for one answer.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct AnswerGrade {
    pub verdict: Verdict,
    /// One or two sentences explaining the verdict.
    pub feedback: String,
    /// The specific points the answer missed (empty on a clean pass).
    #[serde(default)]
    pub missed: Vec<String>,
}

/// The outcome of a sitting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExamResult {
    /// Whether the sitting passed (enough full passes to clear the threshold).
    pub passed: bool,
    /// Per-question grades, in question order.
    pub grades: Vec<AnswerGrade>,
}

impl ExamResult {
    /// The concepts to remediate: every missed point from non-passing answers,
    /// de-duplicated in first-seen order.
    pub fn gaps(&self) -> Vec<String> {
        let mut gaps = Vec::new();
        for grade in &self.grades {
            if grade.verdict == Verdict::Pass {
                continue;
            }
            for point in &grade.missed {
                let point = point.trim();
                if !point.is_empty() && !gaps.iter().any(|g: &String| g == point) {
                    gaps.push(point.to_string());
                }
            }
        }
        gaps
    }
}

/// Generates `cfg.num_questions` open understanding questions from the deck's
/// `% source:`. Blocks until the CLI replies or times out.
pub fn generate_questions(
    deck: &Deck,
    cfg: &ExamConfig,
    ask_cfg: &AskConfig,
) -> Result<Vec<ExamQuestion>> {
    if deck.sources.is_empty() {
        bail!("the deck declares no `% source:` to examine against");
    }
    // The capability gate is enforced inside `generate_questions_from`, which
    // every caller (this fn, `spawn_questions`) routes through. The
    // `ensure_backend_can_examine` call at the CLI/web launch sites is a
    // belt-and-braces pre-flight that fires *before* opening any UI.
    generate_questions_from(&deck.sources, deck.path.parent(), cfg, ask_cfg)
}

/// Checks the configured backend can reach every `% source:` this exam grades
/// against, before any question is generated or side effect taken. A URL source
/// needs a fetch-capable backend; a local source a file-reading one. Called by
/// [`generate_questions`] and by the CLI/web launch sites (before opening the
/// terminal UI) so a capability gap is a clean refusal, not a mid-exam crash.
pub fn ensure_backend_can_examine(deck: &Deck, ask_cfg: &AskConfig) -> Result<()> {
    for source in &deck.sources {
        crate::backend::ensure_source_reachable(ask_cfg, deck::is_url(source))?;
    }
    Ok(())
}

/// Owned-input core of [`generate_questions`]: takes the source list and the
/// base directory directly (not a `&Deck`), so the background
/// [`spawn_questions`] can run it on a thread without borrowing a deck.
///
/// This is the single choke point for all exam question generation (CLI and
/// web), so the capability gate lives here: a URL source needs a fetch-capable
/// backend; a local source needs one that can read files. Both callers
/// (`generate_questions` and `spawn_questions`) inherit the check, so neither
/// path can bypass it.
fn generate_questions_from(
    sources: &[String],
    base: Option<&Path>,
    cfg: &ExamConfig,
    ask_cfg: &AskConfig,
) -> Result<Vec<ExamQuestion>> {
    // Belt-and-braces gate: also checked at CLI/web launch sites (before the
    // UI opens), but enforcing here means the web path can never bypass it.
    for source in sources {
        crate::backend::ensure_source_reachable(ask_cfg, deck::is_url(source))?;
    }
    let prompt = questions_prompt(sources, base, cfg)?;
    let raw = ask::run(&run_config(cfg, ask_cfg), &prompt, &[])?;
    let parsed: QuestionsDto = parse_json(&raw).context("parsing the generated questions")?;
    if parsed.questions.is_empty() {
        bail!("the model returned no questions");
    }
    for q in &parsed.questions {
        if q.prompt.trim().is_empty() || q.points.is_empty() {
            bail!("the model returned a malformed question (empty prompt or no points)");
        }
    }
    Ok(parsed.questions)
}

/// Grades the typed `answers` (one per question, same order) against each
/// question's points, at the given `strictness`, and returns the overall
/// result. A question "passes" when its verdict is [`Verdict::Pass`]; the
/// sitting passes when the fraction of passes is at least `cfg.pass_threshold`.
pub fn grade_answers(
    questions: &[ExamQuestion],
    answers: &[String],
    strictness: Strictness,
    cfg: &ExamConfig,
    ask_cfg: &AskConfig,
) -> Result<ExamResult> {
    if questions.len() != answers.len() {
        bail!(
            "expected {} answers, got {}",
            questions.len(),
            answers.len()
        );
    }
    let prompt = grade_prompt(questions, answers, strictness);
    let raw = ask::run(&run_config(cfg, ask_cfg), &prompt, &[])?;
    let parsed: GradesDto = parse_json(&raw).context("parsing the grades")?;
    if parsed.grades.len() != questions.len() {
        bail!(
            "the model graded {} of {} answers",
            parsed.grades.len(),
            questions.len()
        );
    }
    Ok(ExamResult {
        passed: passed(&parsed.grades, cfg.pass_threshold),
        grades: parsed.grades,
    })
}

/// Grades a learner's **compression** of a trace — their from-memory retrace of
/// the whole path (`description`) — against the path's key `points` (the
/// checkpoints' rubric, in order). Unlike [`grade_answers`], this is ONE
/// holistic judgment of whether the compression re-derives the path's causal
/// chain, not a per-point checklist (a two-sentence gist can't tick every
/// point). Tool-free — the points already paraphrase the source. Blocks until
/// the CLI replies or times out.
pub fn grade_compression(
    description: &str,
    points: &[String],
    compression: &str,
    strictness: Strictness,
    cfg: &ExamConfig,
    ask_cfg: &AskConfig,
) -> Result<ExamResult> {
    let prompt = grade_compression_prompt(description, points, compression, strictness);
    let mut run = run_config(cfg, ask_cfg);
    run.allowed_tools.clear(); // pure reasoning over the supplied text
    let raw = ask::run(&run, &prompt, &[])?;
    let grade: AnswerGrade = parse_json(&raw).context("parsing the compression grade")?;
    Ok(ExamResult {
        passed: grade.verdict == Verdict::Pass,
        grades: vec![grade],
    })
}

/// Turns the missed `gaps` into cards — a cloze/plain card for a missed fact, a
/// `% mode: explain` card for a missed concept — and returns the cleaned
/// deck-format text, ready to append to the deck file.
pub fn remediation_cards(gaps: &[String], cfg: &ExamConfig, ask_cfg: &AskConfig) -> Result<String> {
    if gaps.is_empty() {
        bail!("no gaps to remediate");
    }
    let prompt = remediation_prompt(gaps);
    // Remediation turns the gap list into cards; it needs no web access. Drop the
    // tutor's WebFetch/WebSearch tools so it's a plain text-generation call —
    // faster, and it won't wander off researching the gaps.
    let mut cfg_run = run_config(cfg, ask_cfg);
    cfg_run.allowed_tools.clear();
    let raw = ask::run(&cfg_run, &prompt, &[])?;
    let cards = clean_deck_output(&raw);
    // Every card front starts with `#`; if the reply has none, the model
    // answered in prose instead of emitting cards — treat it as a failure rather
    // than appending the prose to the deck as a bogus "card".
    if !cards.lines().any(|l| l.trim_start().starts_with('#')) {
        bail!("the model replied without any cards — try remediating again");
    }
    Ok(cards)
}

// ── Background runners (for the interactive frontends) ───────────────────────
//
// The three engine calls above are synchronous. The single-threaded web server
// and the TUI event loop run them on a background thread and poll a channel,
// exactly like [`ask::spawn`]. Inputs are owned so the thread is `'static`.

/// Background variant of [`generate_questions`].
pub fn spawn_questions(
    sources: Vec<String>,
    base: Option<PathBuf>,
    cfg: ExamConfig,
    ask_cfg: AskConfig,
) -> Receiver<Result<Vec<ExamQuestion>, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let r = generate_questions_from(&sources, base.as_deref(), &cfg, &ask_cfg)
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(r);
    });
    rx
}

/// Background variant of [`grade_answers`].
pub fn spawn_grade(
    questions: Vec<ExamQuestion>,
    answers: Vec<String>,
    strictness: Strictness,
    cfg: ExamConfig,
    ask_cfg: AskConfig,
) -> Receiver<Result<ExamResult, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let r = grade_answers(&questions, &answers, strictness, &cfg, &ask_cfg)
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(r);
    });
    rx
}

/// Background variant of [`grade_compression`] (the trace exam).
pub fn spawn_grade_compression(
    description: String,
    points: Vec<String>,
    compression: String,
    strictness: Strictness,
    cfg: ExamConfig,
    ask_cfg: AskConfig,
) -> Receiver<Result<ExamResult, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let r = grade_compression(
            &description,
            &points,
            &compression,
            strictness,
            &cfg,
            &ask_cfg,
        )
        .map_err(|e| format!("{e:#}"));
        let _ = tx.send(r);
    });
    rx
}

/// Background variant of [`remediation_cards`].
pub fn spawn_remediation(
    gaps: Vec<String>,
    cfg: ExamConfig,
    ask_cfg: AskConfig,
) -> Receiver<Result<String, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let r = remediation_cards(&gaps, &cfg, &ask_cfg).map_err(|e| format!("{e:#}"));
        let _ = tx.send(r);
    });
    rx
}

/// Turns the cleaned remediation deck-text into virtual cards in `store`
/// (kind [`VirtualKind::Remediation`], `parent = subject`), one per parsed
/// card. Per card: create it if its id is new, revive it if a matching
/// archived card exists, or leave it alone if an active match already exists
/// (dedupe — no schedule reset). Saves the store once after the batch. The
/// deck file is never touched. Returns how many cards were created or
/// revived.
fn store_remediation_cards(
    store: &mut Store,
    subject: &str,
    cards_text: &str,
    now_ms: u64,
) -> Result<usize> {
    let cards = parser::parse_str(subject, cards_text)?;
    if cards.is_empty() {
        bail!("remediation produced no cards to store");
    }

    let mut created_or_revived = 0;
    for card in &cards {
        // A multi-hole cloze sub-card carries a `context` line (the sentence
        // with this hole blanked, others hidden) that tells the user which
        // blank is asked. `VirtualContent` has no separate context field, so
        // fold it into the front to keep the card self-contained.
        let mut front = card.front.clone();
        if !card.context.is_empty() {
            front.push('\n');
            front.push_str(&card.context.join("\n"));
        }
        let id = virtual_id(
            VirtualKind::Remediation,
            subject,
            &remediation_discriminator(&front, &card.back),
        );
        let content = VirtualContent {
            front,
            back: card.back.clone(),
            mode: card.mode,
        };
        match store.get_virtual(&id).map(|vc| vc.retired) {
            None => {
                store.insert_virtual(VirtualCard {
                    id,
                    kind: VirtualKind::Remediation,
                    parent: subject.to_string(),
                    content,
                    state: CardState::new(now_ms),
                    created_ms: now_ms,
                    retired: false,
                });
                created_or_revived += 1;
            }
            Some(true) => {
                store.revive_virtual(&id, now_ms);
                created_or_revived += 1;
            }
            Some(false) => {
                // An active dupe — leave it, no schedule reset.
            }
        }
    }
    store.save()?;
    Ok(created_or_revived)
}

/// The per-card dedupe/revive key for a remediation card: the context-folded
/// `front` (see the caller, [`store_remediation_cards`]) plus the back lines,
/// joined into one string, passed to [`virtual_id`]. The regenerated batch
/// isn't 1:1 with the gaps it was built from (the model is asked to merge
/// overlapping gaps into one card per idea), so content is the only stable
/// per-card key: the same regenerated card reuses its existing schedule, a
/// reworded one gets a fresh id. Unlike `Card::id` (which hashes back-only, to
/// preserve history across a front edit), this includes the front — a virtual
/// card has no edit-history semantics to protect, and the front avoids
/// collisions between two distinct questions sharing a back. Using the
/// context-folded front (rather than the raw, shared `card.front`) also keeps
/// sibling cloze holes for the same answer line — which share `front`, and
/// can share `back` too — from colliding on the same discriminator.
fn remediation_discriminator(front: &str, back: &[String]) -> String {
    let mut s = front.to_string();
    for line in back {
        s.push('\n');
        s.push_str(line);
    }
    s
}

/// The phase of an exam [`Sitting`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Generating questions from the source (background call in flight).
    Generating,
    /// The student is working through the questions one at a time.
    Answering,
    /// Grading the submitted answers (background call in flight).
    Grading,
    /// Showing the graded result; `result().passed` says pass or fail.
    Results,
    /// Generating remediation cards (background call in flight).
    Remediating,
    /// Remediation cards were stored as virtual cards — re-drill and re-sit.
    Remediated,
}

/// The in-flight background call for a [`Sitting`].
enum Pending {
    Questions(Receiver<Result<Vec<ExamQuestion>, String>>),
    Grade(Receiver<Result<ExamResult, String>>),
    Remediation(Receiver<Result<String, String>>),
}

/// What a [`Sitting`] is examining, which selects the grader and a few rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SittingKind {
    /// A fact deck: questions are generated from the `% source:` and graded
    /// against per-question rubrics; a fail can be remediated into cards.
    Source,
    /// A trace: one fixed question (the `% trace:`), answered by retracing the
    /// path in a couple of sentences and graded holistically against the path's
    /// key points. No question generation, no remediation; a fail starts the
    /// re-sit cooldown.
    Trace,
}

/// One in-progress exam sitting — a frontend-agnostic state machine shared by
/// the web server (`serve.rs`) and the TUI (`tui.rs`); the CLI keeps its own
/// linear flow. It owns the exam state and the in-flight background call,
/// spawns each engine step, and on [`poll`](Sitting::poll) transitions and
/// applies the side effects (persist "mastered" on a pass, create remediation
/// virtual cards in the store on confirm). Frontends drive entry/navigation/submit/remediate, call
/// `poll` each tick, and render off `phase()`.
pub struct Sitting {
    kind: SittingKind,
    subject: String,
    strictness: Strictness,
    cfg: ExamConfig,
    ask_cfg: AskConfig,
    phase: Phase,
    questions: Vec<ExamQuestion>,
    answers: Vec<String>,
    current: usize,
    result: Option<ExamResult>,
    pending: Option<Pending>,
    error: Option<String>,
    /// When the in-flight background call started (ms), for an elapsed-time
    /// progress indicator; `None` when idle.
    pending_since: Option<u64>,
}

impl Sitting {
    /// Starts a sitting for `deck` and spawns question generation. The deck
    /// must declare at least one `% source:` (the caller also checks it is
    /// drilled).
    pub fn start(deck: &Deck, strictness: Strictness, cfg: ExamConfig, ask_cfg: AskConfig) -> Self {
        let pending = Pending::Questions(spawn_questions(
            deck.sources.clone(),
            deck.path.parent().map(Path::to_path_buf),
            cfg.clone(),
            ask_cfg.clone(),
        ));
        Self {
            kind: SittingKind::Source,
            subject: deck.subject.clone(),
            strictness,
            cfg,
            ask_cfg,
            phase: Phase::Generating,
            questions: Vec::new(),
            answers: Vec::new(),
            current: 0,
            result: None,
            pending: Some(pending),
            error: None,
            pending_since: Some(crate::time::now_ms()),
        }
    }

    /// Starts a **trace** exam: one fixed question (the `% trace:`
    /// `description`), graded by retracing the path against its `rubric` (the
    /// checkpoints' key points). There is no question generation, so it opens
    /// straight in [`Phase::Answering`] with nothing in flight. `subject` keys
    /// mastery in the store. The caller enforces the re-sit cooldown
    /// ([`crate::store::Store::exam_failed_at`]) before starting.
    pub fn start_trace(
        description: String,
        rubric: Vec<String>,
        subject: String,
        strictness: Strictness,
        cfg: ExamConfig,
        ask_cfg: AskConfig,
    ) -> Self {
        let question = ExamQuestion {
            prompt: description,
            points: rubric,
        };
        Self {
            kind: SittingKind::Trace,
            subject,
            strictness,
            cfg,
            ask_cfg,
            phase: Phase::Answering,
            questions: vec![question],
            answers: vec![String::new()],
            current: 0,
            result: None,
            pending: None,
            error: None,
            pending_since: None,
        }
    }

    /// What this sitting is examining (a fact deck's source, or a trace).
    pub fn kind(&self) -> SittingKind {
        self.kind
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }
    pub fn subject(&self) -> &str {
        &self.subject
    }
    pub fn strictness(&self) -> Strictness {
        self.strictness
    }
    /// A transient error from the last background call, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    /// Whether a background call is in flight.
    pub fn thinking(&self) -> bool {
        self.pending.is_some()
    }
    /// Seconds the in-flight background call has been running, for a progress
    /// indicator; `None` when idle.
    pub fn elapsed_secs(&self) -> Option<u64> {
        self.pending_since
            .map(|since| crate::time::now_ms().saturating_sub(since) / 1000)
    }
    pub fn total(&self) -> usize {
        self.questions.len()
    }
    pub fn current_index(&self) -> usize {
        self.current
    }
    pub fn questions(&self) -> &[ExamQuestion] {
        &self.questions
    }
    pub fn answers(&self) -> &[String] {
        &self.answers
    }
    pub fn result(&self) -> Option<&ExamResult> {
        self.result.as_ref()
    }
    /// The current question (in [`Phase::Answering`]).
    pub fn question(&self) -> Option<&ExamQuestion> {
        self.questions.get(self.current)
    }
    /// The answer typed for the current question so far.
    pub fn answer(&self) -> &str {
        self.answers
            .get(self.current)
            .map(String::as_str)
            .unwrap_or("")
    }
    /// `true` if the current question is the last one.
    pub fn on_last(&self) -> bool {
        !self.questions.is_empty() && self.current + 1 == self.questions.len()
    }

    /// Replaces the current question's answer (no-op outside
    /// [`Phase::Answering`]).
    pub fn set_answer(&mut self, text: String) {
        if self.phase == Phase::Answering
            && let Some(slot) = self.answers.get_mut(self.current)
        {
            *slot = text;
        }
    }
    /// Appends a character to the current answer (char-by-char TUI input).
    pub fn push_char(&mut self, c: char) {
        if self.phase == Phase::Answering
            && let Some(slot) = self.answers.get_mut(self.current)
        {
            slot.push(c);
        }
    }
    /// Removes the last character of the current answer.
    pub fn pop_char(&mut self) {
        if self.phase == Phase::Answering
            && let Some(slot) = self.answers.get_mut(self.current)
        {
            slot.pop();
        }
    }
    pub fn next(&mut self) {
        if self.current + 1 < self.questions.len() {
            self.current += 1;
        }
    }
    pub fn prev(&mut self) {
        self.current = self.current.saturating_sub(1);
    }
    pub fn goto(&mut self, i: usize) {
        if i < self.questions.len() {
            self.current = i;
        }
    }

    /// Submits all answers for grading ([`Phase::Answering`] →
    /// [`Phase::Grading`]). A `Source` exam grades each answer against its
    /// rubric; a `Trace` exam grades the single compression holistically against
    /// the path.
    pub fn submit(&mut self) {
        if self.phase != Phase::Answering {
            return;
        }
        self.error = None;
        let rx = match self.kind {
            SittingKind::Source => spawn_grade(
                self.questions.clone(),
                self.answers.clone(),
                self.strictness,
                self.cfg.clone(),
                self.ask_cfg.clone(),
            ),
            SittingKind::Trace => {
                let q = &self.questions[0];
                spawn_grade_compression(
                    q.prompt.clone(),
                    q.points.clone(),
                    self.answers[0].clone(),
                    self.strictness,
                    self.cfg.clone(),
                    self.ask_cfg.clone(),
                )
            }
        };
        self.pending = Some(Pending::Grade(rx));
        self.pending_since = Some(crate::time::now_ms());
        self.phase = Phase::Grading;
    }

    /// The missed gaps from the result (empty until graded).
    pub fn gaps(&self) -> Vec<String> {
        self.result
            .as_ref()
            .map(ExamResult::gaps)
            .unwrap_or_default()
    }

    /// Whether remediation is offerable (failed result with gaps to fix). Never
    /// for a `Trace` exam: a trace deck is a path of checkpoints, not a card
    /// pile — a failed compression is re-walked, not remediated into cards.
    pub fn can_remediate(&self) -> bool {
        self.kind == SittingKind::Source
            && self.phase == Phase::Results
            && self.result.as_ref().is_some_and(|r| !r.passed)
            && !self.gaps().is_empty()
    }

    /// Generates remediation cards for the gaps and stores them as virtual cards
    /// ([`Phase::Results`] → [`Phase::Remediating`]). No-op if nothing to fix.
    pub fn remediate(&mut self) {
        if !self.can_remediate() {
            return;
        }
        self.error = None;
        self.pending = Some(Pending::Remediation(spawn_remediation(
            self.gaps(),
            self.cfg.clone(),
            self.ask_cfg.clone(),
        )));
        self.pending_since = Some(crate::time::now_ms());
        self.phase = Phase::Remediating;
    }

    /// Drains a finished background call and advances the phase, applying side
    /// effects: on a passing grade, persist "mastered" and save `store`; on
    /// remediation, create/dedupe/revive virtual cards in `store` (the deck
    /// file is never touched). Returns `true` when the phase advanced.
    pub fn poll(&mut self, store: &mut Store, now_ms: u64) -> bool {
        let reply = match &self.pending {
            None => return false,
            Some(Pending::Questions(rx)) => match rx.try_recv() {
                Ok(r) => Reply::Questions(r),
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => Reply::Questions(Err(thread_gone())),
            },
            Some(Pending::Grade(rx)) => match rx.try_recv() {
                Ok(r) => Reply::Grade(r),
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => Reply::Grade(Err(thread_gone())),
            },
            Some(Pending::Remediation(rx)) => match rx.try_recv() {
                Ok(r) => Reply::Remediation(r),
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => Reply::Remediation(Err(thread_gone())),
            },
        };
        self.pending = None;
        self.pending_since = None;
        match reply {
            Reply::Questions(Ok(qs)) => {
                self.answers = vec![String::new(); qs.len()];
                self.questions = qs;
                self.current = 0;
                self.phase = Phase::Answering;
            }
            // Generation failed before any questions: stays Generating, but with
            // an error and nothing in flight; the frontend offers to close.
            Reply::Questions(Err(e)) => self.error = Some(e),
            Reply::Grade(Ok(result)) => {
                if result.passed {
                    store.set_deck_mastered(&self.subject, now_ms);
                    let _ = store.save();
                } else if self.kind == SittingKind::Trace {
                    // A failed trace exam starts the re-sit cooldown, so the
                    // graded feedback can't be pasted straight back into the one
                    // fixed question.
                    store.set_exam_failed(&self.subject, now_ms);
                    let _ = store.save();
                }
                self.result = Some(result);
                self.phase = Phase::Results;
            }
            // Grading failed: back to answering so the student can resubmit.
            Reply::Grade(Err(e)) => {
                self.error = Some(e);
                self.phase = Phase::Answering;
            }
            Reply::Remediation(Ok(cards)) => {
                match store_remediation_cards(store, &self.subject, &cards, now_ms) {
                    Ok(_n) => self.phase = Phase::Remediated,
                    Err(e) => {
                        self.error = Some(format!("{e}"));
                        self.phase = Phase::Results;
                    }
                }
            }
            Reply::Remediation(Err(e)) => {
                self.error = Some(e);
                self.phase = Phase::Results;
            }
        }
        true
    }
}

/// A drained background reply, used inside [`Sitting::poll`].
enum Reply {
    Questions(Result<Vec<ExamQuestion>, String>),
    Grade(Result<ExamResult, String>),
    Remediation(Result<String, String>),
}

fn thread_gone() -> String {
    "the exam helper exited unexpectedly".to_string()
}

/// Milliseconds left on a failed trace exam's re-sit cooldown, or `None` if it
/// can be sat now — it never failed, the cooldown has elapsed, or the cooldown
/// is disabled (`cooldown_secs == 0`). The launch sites (CLI `exam`, the web
/// `Take exam`) gate on this so the graded feedback can't be pasted straight
/// back into the one fixed trace question.
pub fn cooldown_remaining_ms(
    store: &Store,
    subject: &str,
    cooldown_secs: u64,
    now_ms: u64,
) -> Option<u64> {
    if cooldown_secs == 0 {
        return None;
    }
    let until = store
        .exam_failed_at(subject)?
        .saturating_add(cooldown_secs.saturating_mul(1000));
    (until > now_ms).then(|| until - now_ms)
}

/// `true` when the fraction of full passes meets the threshold.
fn passed(grades: &[AnswerGrade], threshold: f64) -> bool {
    if grades.is_empty() {
        return false;
    }
    let passes = grades.iter().filter(|g| g.verdict == Verdict::Pass).count();
    (passes as f64) / (grades.len() as f64) >= threshold
}

/// The CLI runner config for the exam: the ask command/permission/tools with
/// the exam's own model and (longer) timeout.
fn run_config(cfg: &ExamConfig, ask_cfg: &AskConfig) -> AskConfig {
    AskConfig {
        model: cfg.model.clone().or_else(|| ask_cfg.model.clone()),
        timeout_secs: cfg.timeout_secs,
        cwd: None,
        source_access: false,
        ..ask_cfg.clone()
    }
}

/// Renders the deck's sources into a prompt section: URLs become WebFetch
/// instructions, local files are read and embedded (bounded). Relative file
/// paths resolve against the deck file's folder.
fn source_section(sources: &[String], base: Option<&Path>) -> Result<String> {
    let mut urls = Vec::new();
    let mut files = Vec::new();
    for src in sources {
        if deck::is_url(src) {
            urls.push(src.clone());
            continue;
        }
        // A value may name several files joined with " + " (a shorthand the
        // generator sometimes emits, e.g. `README.md + src/lib.rs`); read each
        // resolved path, skipping any that don't exist rather than failing.
        for path in crate::trace::source_paths(src, base) {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    let label = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    let (truncated_text, was_truncated) = truncate(&text);
                    if was_truncated {
                        eprintln!(
                            "note: `{}` is larger than {} KB and was truncated for the exam prompt",
                            label,
                            MAX_SOURCE_BYTES / 1_000
                        );
                    }
                    files.push((label, truncated_text));
                }
                Err(e) => eprintln!(
                    "warning: skipping unreadable `% source:` {}: {e}",
                    path.display()
                ),
            }
        }
    }
    if urls.is_empty() && files.is_empty() {
        bail!("none of the deck's `% source:` paths could be read to examine against");
    }

    let mut out = String::new();
    if !urls.is_empty() {
        out.push_str(
            "Read these source pages with the WebFetch tool (fetch each once) — \
             they are the ground truth for this exam:\n",
        );
        for url in &urls {
            out.push_str("  - ");
            out.push_str(url);
            out.push('\n');
        }
    }
    for (label, text) in &files {
        out.push_str(&format!(
            "\nSource file `{label}` (the ground truth for this exam):\n<<<SOURCE\n{text}\nSOURCE\n"
        ));
    }
    Ok(out)
}

/// Truncates source text to [`MAX_SOURCE_BYTES`] on a char boundary, appending
/// a marker when it had to cut. Returns the (possibly truncated) text and a
/// flag indicating whether truncation occurred.
fn truncate(text: &str) -> (String, bool) {
    if text.len() <= MAX_SOURCE_BYTES {
        return (text.to_string(), false);
    }
    let mut end = MAX_SOURCE_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (
        format!("{}\n[... source truncated ...]", &text[..end]),
        true,
    )
}

/// Builds the question-generation prompt from the deck's sources.
fn questions_prompt(sources: &[String], base: Option<&Path>, cfg: &ExamConfig) -> Result<String> {
    let sources = source_section(sources, base)?;
    let mut prompt = format!(
        "You are a strict examiner writing an oral exam that verifies a student \
         truly UNDERSTANDS a topic — not whether they memorized isolated facts.\n\n\
         {sources}\n\
         Write exactly {n} open-ended questions grounded in the source above. \
         Favor APPLICATION (\"given X, what happens / what would you do?\") and \
         CONNECTIONS (how ideas relate, contrast or build on each other) over \
         plain recall. Each question must be answerable from the source and \
         demand reasoning in the student's own words.\n\n\
         For each question, list the key points a correct answer MUST contain — \
         the specific, source-grounded facts or reasoning steps you will grade \
         against. Be concrete; these are the rubric.\n\n\
         Output ONLY JSON in exactly this shape, no prose, no code fences:\n\
         {{\"questions\": [{{\"prompt\": \"...\", \"points\": [\"...\", \"...\"]}}]}}",
        n = cfg.num_questions,
    );
    if let Some(extra) = cfg
        .extra
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        prompt.push_str("\n\nAdditional instructions:\n");
        prompt.push_str(extra);
    }
    Ok(prompt)
}

/// The grading criteria for a strictness level — the part of the grade prompt
/// that decides how generously a typed answer is judged against the rubric and
/// what counts as a `missed` point (which drives remediation).
fn strictness_criteria(strictness: Strictness) -> &'static str {
    match strictness {
        Strictness::Strict => {
            "\
Grade strictly for COMPLETENESS — treat the rubric as a checklist. The answer \
passes only if it covers EVERY key point (wording may differ, but each point \
must be present). List in `missed` every key point the answer omits or gets \
wrong; here an omitted point IS a gap, because this material requires recalling \
all of it.\n\
- \"pass\": every key point is present.\n\
- \"partial\": some present, others missing or wrong.\n\
- \"fail\": most key points missing or wrong."
        }
        Strictness::Balanced => {
            "\
Grade for UNDERSTANDING, not completeness of phrasing. A key point counts as \
covered if the answer shows the student grasps it — briefly, in their own \
words, or by clear implication. Put a point in `missed` ONLY if the answer is \
wrong about it or shows the student genuinely does not grasp it — NEVER merely \
because they didn't spell it out or skipped a secondary detail.\n\
- \"pass\": the answer demonstrates the question's core idea.\n\
- \"partial\": part understood, but something important is wrong or absent.\n\
- \"fail\": little or no understanding."
        }
        Strictness::Lenient => {
            "\
Grade generously — the goal is only to catch real misunderstandings. Give the \
student the benefit of the doubt on anything plausibly correct or partially \
stated. Put a point in `missed` ONLY if the answer is clearly wrong about it or \
the question was essentially not answered.\n\
- \"pass\": broadly correct, even if thin.\n\
- \"partial\": partly right but with a clear error.\n\
- \"fail\": wrong or essentially unanswered."
        }
    }
}

/// Builds the grading prompt: the strictness criteria, then each question, its
/// rubric points and the student's answer.
fn grade_prompt(questions: &[ExamQuestion], answers: &[String], strictness: Strictness) -> String {
    let mut prompt = String::from(
        "You are an examiner grading an understanding exam. For each question you \
         are given the key points a correct answer covers (the rubric) and the \
         student's answer.\n",
    );
    prompt.push_str(strictness_criteria(strictness));
    prompt.push_str(
        "\nKeep `feedback` to one or two sentences, and cite the specific missed \
         point(s) in `missed` per the rule above.\n\n",
    );
    for (i, (q, a)) in questions.iter().zip(answers).enumerate() {
        prompt.push_str(&format!("Question {}: {}\n", i + 1, q.prompt));
        prompt.push_str("Key points (rubric):\n");
        for point in &q.points {
            prompt.push_str("  - ");
            prompt.push_str(point);
            prompt.push('\n');
        }
        let answer = a.trim();
        let answer = if answer.is_empty() {
            "(no answer given)"
        } else {
            answer
        };
        prompt.push_str("Student's answer: ");
        prompt.push_str(answer);
        prompt.push_str("\n\n");
    }
    prompt.push_str(
        "Output ONLY JSON in exactly this shape, one grade per question in order, \
         no prose, no code fences:\n\
         {\"grades\": [{\"verdict\": \"pass|partial|fail\", \"feedback\": \"...\", \
         \"missed\": [\"...\"]}]}",
    );
    prompt
}

/// The grading criteria for the trace **compression** at a strictness level —
/// the holistic counterpart of [`strictness_criteria`]. The compression is one
/// short retrace of the whole path, so it's judged on re-deriving the causal
/// chain, not on ticking every rubric point.
fn compression_strictness_criteria(strictness: Strictness) -> &'static str {
    match strictness {
        Strictness::Strict => {
            "\
Grade strictly: the retrace must name every load-bearing step and the causal \
link from each to the next. A missing step, a wrong order, or a hand-waved \"and \
then it works\" is a gap.\n\
- \"pass\": the whole chain is re-derived — each step and link present.\n\
- \"partial\": the gist is right but a step or a link is missing or wrong.\n\
- \"fail\": the chain is mostly absent, out of order, or wrong."
        }
        Strictness::Balanced => {
            "\
Grade for whether the learner can RE-DERIVE the path: the main steps and how \
each leads to the next, in their own words. Forgive omitted minor detail and \
loose wording — a two-sentence gist that captures the causal spine passes. Mark \
a point missed only when a load-bearing step or link is absent or wrong.\n\
- \"pass\": the causal spine is re-derived end to end.\n\
- \"partial\": part of the chain is there, but a key step or link is missing or \
wrong.\n\
- \"fail\": little of the path's chain is reconstructed."
        }
        Strictness::Lenient => {
            "\
Grade generously: pass if the compression broadly captures the path's shape — \
roughly where it starts, the main move, and where it ends — even if thin. Fail \
only a retrace that is clearly wrong or essentially absent.\n\
- \"pass\": broadly traces the path, even thinly.\n\
- \"partial\": gestures at the path but with a clear error or gap.\n\
- \"fail\": wrong or essentially no retrace."
        }
    }
}

/// Builds the trace-exam grading prompt: the path question, its key points (the
/// ground-truth rubric, in order) and the learner's compression — asking for one
/// holistic `pass|partial|fail` on whether the retrace re-derives the path.
fn grade_compression_prompt(
    description: &str,
    points: &[String],
    compression: &str,
    strictness: Strictness,
) -> String {
    let mut prompt = String::from(
        "You are grading the final exam of a guided predict-and-verify walk \
         through a source (a \"trace\"). The learner walked the path hop by hop; \
         now, from memory, they RETRACE THE WHOLE PATH in a sentence or two. This \
         compression IS the exam: it verifies they can RE-DERIVE the path (the \
         steps and how they connect), not merely recognize each step.\n\n\
         The path answers this question:\n",
    );
    prompt.push_str(description.trim());
    prompt.push_str(
        "\n\nA faithful retrace re-derives these load-bearing points, in order \
         (drawn from the path's checkpoints — the ground truth):\n",
    );
    for point in points {
        prompt.push_str("  - ");
        prompt.push_str(point);
        prompt.push('\n');
    }
    prompt.push_str("\nThe learner's compression:\n");
    let answer = compression.trim();
    prompt.push_str(if answer.is_empty() {
        "(no answer given)"
    } else {
        answer
    });
    prompt.push_str("\n\n");
    prompt.push_str(compression_strictness_criteria(strictness));
    prompt.push_str(
        "\nJudge the CAUSAL CHAIN, not verbatim coverage — a short retrace that \
         re-derives the spine passes even if it omits detail, while a confident \
         retrace that gets the mechanism wrong does not. Put the specific missing \
         or wrong steps in `missed`. Keep `feedback` to one or two sentences.\n\
         Output ONLY JSON in exactly this shape, no prose, no code fences:\n\
         {\"verdict\": \"pass|partial|fail\", \"feedback\": \"...\", \"missed\": [\"...\"]}",
    );
    prompt
}

/// Builds the remediation prompt: turn missed concepts into cards, choosing the
/// type per gap — a cloze/plain card for a missed fact or term, a
/// `% mode: explain` understanding card for a missed concept or connection.
fn remediation_prompt(gaps: &[String]) -> String {
    let mut prompt = String::from(
        "A student failed an understanding exam on the concepts below. Turn them \
         into spaced-repetition cards that close each gap.\n\n\
         FIRST consolidate the list: some of these concepts overlap or restate one \
         another. MERGE overlapping concepts and write ONE card per DISTINCT idea — \
         never two cards that test the same thing. Fewer, non-overlapping cards are \
         better than covering every line verbatim; only merge true duplicates, \
         never drop a distinct idea.\n\n\
         CHOOSE THE CARD TYPE per gap, by what the gap actually is:\n\
         - A missed FACT or TERM (a definition, a value, what lives where) -> a \
         cheap recall card. Prefer a cloze: a `#?` front with a short instruction, \
         and an indented answer line that states the fact with each hidden span \
         wrapped in {{double curly braces}} (a lone single brace is literal). If \
         there is no natural word to blank out, use a plain `# ` card with the \
         answer on the indented line below instead.\n\
         - A missed CONCEPT, MECHANISM or CONNECTION (a \"why\", \"how\" or \"what \
         happens if\") -> an understanding card: a `# ` open prompt, then the next \
         line exactly `% mode: explain`, then indented key points a good answer \
         covers. This forces the student to re-derive the idea, which is what the \
         exam re-tests. When unsure, prefer the understanding card.\n\n\
         CONCEPTS THE STUDENT MISSED:\n",
    );
    for gap in gaps {
        prompt.push_str("  - ");
        prompt.push_str(gap);
        prompt.push('\n');
    }
    prompt.push_str(
        "\nFORMAT — a plain-text deck, cards one after another. A card's answer is \
         on the indented (tab) line(s) below its front; an indented `! ` line after \
         it adds an optional note (a caveat, example or why it matters). One \
         example of each card type:\n\
         #? Recall how a String is laid out in memory.\n\
         \tA String stores a {{pointer}}, {{length}} and {{capacity}} on the stack, \
         and its bytes live on the {{heap}}.\n\
         # What does `drop` do for a String, and when?\n\
         \tIt returns the String's heap buffer to the allocator, at the end of the \
         owning scope.\n\
         # Why does moving a String invalidate the original binding?\n\
         % mode: explain\n\
         \tBoth bindings would otherwise point at the same heap buffer.\n\
         \tDropping both would free it twice (a double free).\n\
         \tSo Rust invalidates the source instead of allowing two owners.\n\
         \t! A move, not a shallow copy you can keep using.\n\n\
         Before finishing, re-read your cards as a set and merge any two that test \
         the same idea, so every card is distinct.\n\
         Output ONLY the deck text — no markdown code fences, no preamble, no \
         closing remarks.",
    );
    prompt
}

/// Extracts the JSON object from a model reply that may be wrapped in code
/// fences or surrounded by prose: the substring from the first `{` to the last
/// `}`. Falls back to the trimmed input.
fn extract_json(raw: &str) -> &str {
    match (raw.find('{'), raw.rfind('}')) {
        (Some(start), Some(end)) if end > start => &raw[start..=end],
        _ => raw.trim(),
    }
}

/// Parses `raw` (possibly fenced/with preamble) into `T`.
fn parse_json<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T> {
    let json = extract_json(raw);
    serde_json::from_str(json)
        .with_context(|| format!("the model did not return valid JSON:\n{json}"))
}

/// Strips code fences / leading commentary from generated deck text, like
/// [`crate::generate`] does: a deck starts with a `%` comment or a `#` card
/// front, and trailing blank/fence lines are dropped.
fn clean_deck_output(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let Some(start) = lines.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with('%') || t.starts_with('#')
    }) else {
        return raw.trim().to_string();
    };
    let mut end = lines.len();
    while end > start + 1 {
        let t = lines[end - 1].trim();
        if t.is_empty() || t.starts_with("```") {
            end -= 1;
        } else {
            break;
        }
    }
    lines[start..end].join("\n")
}

#[derive(Debug, Deserialize)]
struct QuestionsDto {
    questions: Vec<ExamQuestion>,
}

#[derive(Debug, Deserialize)]
struct GradesDto {
    grades: Vec<AnswerGrade>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck_with_sources(srcs: &[&str]) -> Deck {
        Deck {
            path: std::path::PathBuf::from("/tmp/d.txt"),
            subject: "d.txt".to_string(),
            cards: Vec::new(),
            links: Vec::new(),
            requires: Vec::new(),
            sources: srcs.iter().map(|s| s.to_string()).collect(),
            settings: Default::default(),
            title: None,
            trace: None,
        }
    }

    #[test]
    fn questions_prompt_carries_source_strictness_and_json_shape() {
        let deck = deck_with_sources(&["https://example.org/ownership"]);
        let p =
            questions_prompt(&deck.sources, deck.path.parent(), &ExamConfig::default()).unwrap();
        assert!(p.contains("https://example.org/ownership"));
        assert!(p.contains("WebFetch"));
        assert!(p.contains("strict examiner"));
        assert!(p.contains("APPLICATION"));
        assert!(p.contains("\"questions\""));
        assert!(p.contains("exactly 5 open-ended questions"));
    }

    #[test]
    fn questions_prompt_honors_num_questions_and_extra() {
        let deck = deck_with_sources(&["https://x"]);
        let cfg = ExamConfig {
            num_questions: 3,
            extra: Some("Focus on lifetimes.".to_string()),
            ..ExamConfig::default()
        };
        let p = questions_prompt(&deck.sources, deck.path.parent(), &cfg).unwrap();
        assert!(p.contains("exactly 3 open-ended questions"));
        assert!(p.contains("Additional instructions:"));
        assert!(p.contains("Focus on lifetimes."));
    }

    #[test]
    fn grade_prompt_includes_rubric_and_answers() {
        let qs = vec![ExamQuestion {
            prompt: "Why move?".to_string(),
            points: vec!["avoids double free".to_string()],
        }];
        let p = grade_prompt(
            &qs,
            &["because ownership".to_string()],
            Strictness::Balanced,
        );
        assert!(p.contains("Question 1: Why move?"));
        assert!(p.contains("avoids double free"));
        assert!(p.contains("because ownership"));
        assert!(p.contains("examiner"));
        assert!(p.contains("\"grades\""));
    }

    #[test]
    fn grade_prompt_marks_empty_answers() {
        let qs = vec![ExamQuestion {
            prompt: "Q".to_string(),
            points: vec!["p".to_string()],
        }];
        let p = grade_prompt(&qs, &["   ".to_string()], Strictness::Balanced);
        assert!(p.contains("(no answer given)"));
    }

    #[test]
    fn grade_prompt_criteria_vary_by_strictness() {
        let qs = vec![ExamQuestion {
            prompt: "Q".to_string(),
            points: vec!["p".to_string()],
        }];
        let a = vec!["a".to_string()];
        let strict = grade_prompt(&qs, &a, Strictness::Strict);
        let balanced = grade_prompt(&qs, &a, Strictness::Balanced);
        let lenient = grade_prompt(&qs, &a, Strictness::Lenient);
        // Strict demands completeness; balanced judges understanding; lenient is
        // generous. Each carries language the others don't.
        assert!(strict.contains("COMPLETENESS"));
        assert!(strict.contains("EVERY key point"));
        assert!(balanced.contains("UNDERSTANDING, not completeness"));
        assert!(!balanced.contains("COMPLETENESS — treat the rubric"));
        assert!(lenient.contains("benefit of the doubt"));
    }

    #[test]
    fn remediation_prompt_asks_for_explain_cards() {
        let p = remediation_prompt(&["the aliasing rule".to_string()]);
        assert!(p.contains("the aliasing rule"));
        assert!(p.contains("% mode: explain"));
        assert!(p.contains("Output ONLY the deck text"));
    }

    #[test]
    fn remediation_prompt_picks_card_type_per_gap() {
        let p = remediation_prompt(&["x".to_string()]);
        // Facts get a cheap recall card (cloze/plain); concepts get explain.
        assert!(p.contains("missed FACT or TERM"));
        assert!(p.contains("missed CONCEPT"));
        assert!(p.contains("#?")); // cloze front for facts
        assert!(p.contains("double curly braces"));
        assert!(p.contains("% mode: explain")); // understanding card for concepts
    }

    #[test]
    fn remediation_prompt_asks_to_dedup_overlapping_gaps() {
        let p = remediation_prompt(&[
            "String stores a pointer, length and capacity on the stack".to_string(),
            "A String keeps its bytes on the heap and a pointer on the stack".to_string(),
        ]);
        // Both near-duplicate gaps still appear, but the model is told to merge.
        assert!(p.contains("String stores a pointer"));
        assert!(p.contains("keeps its bytes on the heap"));
        assert!(p.contains("MERGE overlapping concepts"));
        assert!(p.contains("ONE card per DISTINCT idea"));
        assert!(p.contains("merge any two that test the same idea"));
    }

    #[test]
    fn parse_questions_from_fenced_json() {
        let raw = "Here you go:\n```json\n{\"questions\": [{\"prompt\": \"Q1\", \
                   \"points\": [\"a\", \"b\"]}]}\n```";
        let dto: QuestionsDto = parse_json(raw).unwrap();
        assert_eq!(1, dto.questions.len());
        assert_eq!("Q1", dto.questions[0].prompt);
        assert_eq!(vec!["a", "b"], dto.questions[0].points);
    }

    #[test]
    fn parse_grades_with_verdicts() {
        let raw = "{\"grades\": [\
            {\"verdict\": \"pass\", \"feedback\": \"good\", \"missed\": []},\
            {\"verdict\": \"fail\", \"feedback\": \"no\", \"missed\": [\"x\"]}]}";
        let dto: GradesDto = parse_json(raw).unwrap();
        assert_eq!(Verdict::Pass, dto.grades[0].verdict);
        assert_eq!(Verdict::Fail, dto.grades[1].verdict);
        assert_eq!(vec!["x"], dto.grades[1].missed);
    }

    #[test]
    fn missed_defaults_when_absent() {
        let raw = "{\"grades\": [{\"verdict\": \"pass\", \"feedback\": \"ok\"}]}";
        let dto: GradesDto = parse_json(raw).unwrap();
        assert!(dto.grades[0].missed.is_empty());
    }

    #[test]
    fn invalid_json_is_a_clear_error() {
        let err = parse_json::<GradesDto>("not json at all").unwrap_err();
        assert!(format!("{err:#}").contains("valid JSON"));
    }

    #[test]
    fn threshold_all_pass_passes() {
        let grades = vec![
            AnswerGrade {
                verdict: Verdict::Pass,
                feedback: String::new(),
                missed: Vec::new(),
            },
            AnswerGrade {
                verdict: Verdict::Pass,
                feedback: String::new(),
                missed: Vec::new(),
            },
        ];
        assert!(passed(&grades, 1.0));
    }

    #[test]
    fn threshold_one_partial_fails_strict() {
        let grades = vec![
            AnswerGrade {
                verdict: Verdict::Pass,
                feedback: String::new(),
                missed: Vec::new(),
            },
            AnswerGrade {
                verdict: Verdict::Partial,
                feedback: String::new(),
                missed: vec!["m".to_string()],
            },
        ];
        assert!(!passed(&grades, 1.0));
        // A looser threshold (half) would let it through.
        assert!(passed(&grades, 0.5));
    }

    #[test]
    fn gaps_collects_missed_from_non_passes_deduped() {
        let result = ExamResult {
            passed: false,
            grades: vec![
                AnswerGrade {
                    verdict: Verdict::Pass,
                    feedback: String::new(),
                    missed: vec!["ignored (passed)".to_string()],
                },
                AnswerGrade {
                    verdict: Verdict::Fail,
                    feedback: String::new(),
                    missed: vec!["dup".to_string(), "unique".to_string()],
                },
                AnswerGrade {
                    verdict: Verdict::Partial,
                    feedback: String::new(),
                    missed: vec!["dup".to_string()],
                },
            ],
        };
        assert_eq!(vec!["dup", "unique"], result.gaps());
    }

    #[test]
    fn clean_deck_output_strips_fences() {
        let raw = "```text\n# Q\n% mode: explain\n\tA\n```";
        assert_eq!("# Q\n% mode: explain\n\tA", clean_deck_output(raw));
    }

    use crate::answer::Mode;
    use crate::testutil::{ask_config, exec_lock, fake_cli, fake_reply};

    #[test]
    fn verdict_labels() {
        assert_eq!("PASS", Verdict::Pass.label());
        assert_eq!("PARTIAL", Verdict::Partial.label());
        assert_eq!("FAIL", Verdict::Fail.label());
    }

    #[test]
    fn generate_questions_rejects_a_deck_without_a_source() {
        let deck = deck_with_sources(&[]);
        let err = generate_questions(
            &deck,
            &ExamConfig::default(),
            &ask_config(std::path::Path::new("unused")),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("no `% source:`"));
    }

    /// The web exam path calls `spawn_questions` → `generate_questions_from`
    /// directly, bypassing `generate_questions` and the CLI pre-flight. This
    /// test verifies the gate inside `generate_questions_from` fires for a
    /// codex-backed URL-source exam, so both paths get the clean capability
    /// message rather than a raw CLI failure.
    #[test]
    fn generate_questions_from_rejects_url_source_on_fetch_incapable_backend() {
        use crate::config::BackendKind;
        // Codex is a read-only backend (can_fetch_web() == false); a URL
        // source with codex must refuse cleanly before any CLI call is made.
        let ask_cfg = AskConfig {
            backend: BackendKind::Codex,
            // The command points at nothing — the gate must fire before it's
            // invoked, so an unreachable command is fine.
            command: "/dev/null".to_string(),
            timeout_secs: 5,
            ..AskConfig::default()
        };
        // Drive the shared private path via `spawn_questions` (as the web
        // server does) and drain the receiver — no exec_lock needed because
        // the gate fires before any subprocess is forked.
        let rx = spawn_questions(
            vec!["https://example.org/doc".to_string()],
            None,
            ExamConfig::default(),
            ask_cfg,
        );
        let err = rx.recv().unwrap().unwrap_err();
        // The message must name the backend and point to the fix, not surface
        // a raw CLI invocation failure.
        assert!(
            err.contains("codex") && err.contains("can't fetch a url"),
            "expected capability-refusal message, got: {err}"
        );
    }

    #[test]
    fn generate_questions_rejects_an_empty_reply() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), "{\"questions\":[]}");
        let deck = deck_with_sources(&["https://x"]);
        let err = generate_questions(&deck, &ExamConfig::default(), &ask_config(&cli)).unwrap_err();
        assert!(format!("{err:#}").contains("no questions"));
    }

    #[test]
    fn generate_questions_rejects_a_malformed_question() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(
            dir.path(),
            "{\"questions\":[{\"prompt\":\"\",\"points\":[]}]}",
        );
        let deck = deck_with_sources(&["https://x"]);
        let err = generate_questions(&deck, &ExamConfig::default(), &ask_config(&cli)).unwrap_err();
        assert!(format!("{err:#}").contains("malformed"));
    }

    #[test]
    fn grade_answers_rejects_a_count_mismatch() {
        let qs = vec![
            ExamQuestion {
                prompt: "Q1".to_string(),
                points: vec!["p".to_string()],
            },
            ExamQuestion {
                prompt: "Q2".to_string(),
                points: vec!["p".to_string()],
            },
        ];
        let err = grade_answers(
            &qs,
            &["only one".to_string()],
            Strictness::Balanced,
            &ExamConfig::default(),
            &ask_config(std::path::Path::new("unused")),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("expected 2 answers, got 1"));
    }

    #[test]
    fn grade_answers_rejects_a_wrong_grade_count() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // One grade for two questions.
        let cli = fake_reply(
            dir.path(),
            "{\"grades\":[{\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]}]}",
        );
        let qs = vec![
            ExamQuestion {
                prompt: "Q1".to_string(),
                points: vec!["p".to_string()],
            },
            ExamQuestion {
                prompt: "Q2".to_string(),
                points: vec!["p".to_string()],
            },
        ];
        let err = grade_answers(
            &qs,
            &["a".to_string(), "b".to_string()],
            Strictness::Balanced,
            &ExamConfig::default(),
            &ask_config(&cli),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("graded 1 of 2"));
    }

    #[test]
    fn remediation_cards_rejects_no_gaps() {
        let err = remediation_cards(
            &[],
            &ExamConfig::default(),
            &ask_config(std::path::Path::new("unused")),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("no gaps"));
    }

    #[test]
    fn generate_questions_end_to_end() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let json = "{\"questions\":[{\"prompt\":\"Why move?\",\
                    \"points\":[\"avoids double free\"]}]}";
        let cli = fake_reply(dir.path(), json);
        let deck = deck_with_sources(&["https://x"]);
        let cfg = ExamConfig {
            num_questions: 1,
            ..ExamConfig::default()
        };
        let qs = generate_questions(&deck, &cfg, &ask_config(&cli)).unwrap();
        assert_eq!(1, qs.len());
        assert_eq!("Why move?", qs[0].prompt);
        assert_eq!(vec!["avoids double free"], qs[0].points);
    }

    #[test]
    fn grade_answers_end_to_end_pass() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let json = "{\"grades\":[{\"verdict\":\"pass\",\"feedback\":\"good\",\"missed\":[]}]}";
        let cli = fake_reply(dir.path(), json);
        let qs = vec![ExamQuestion {
            prompt: "Q".to_string(),
            points: vec!["p".to_string()],
        }];
        let result = grade_answers(
            &qs,
            &["my answer".to_string()],
            Strictness::Balanced,
            &ExamConfig::default(),
            &ask_config(&cli),
        )
        .unwrap();
        assert!(result.passed);
        assert_eq!(Verdict::Pass, result.grades[0].verdict);
    }

    #[test]
    fn remediation_cards_end_to_end() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // A fenced deck; clean_deck_output must strip the fences.
        let cli = fake_reply(
            dir.path(),
            "```text\n# Why?\n% mode: explain\n  point\n```\n",
        );
        let cards = remediation_cards(
            &["the gap".to_string()],
            &ExamConfig::default(),
            &ask_config(&cli),
        )
        .unwrap();
        assert_eq!("# Why?\n% mode: explain\n  point", cards);
    }

    #[test]
    fn remediation_cards_rejects_a_reply_without_cards() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // The model answered in prose (no `#` card front) instead of emitting
        // cards: a failure, not a silently-appended bogus "card".
        let cli = fake_reply(dir.path(), "Sure, here is some advice on those concepts.");
        let err = remediation_cards(
            &["the gap".to_string()],
            &ExamConfig::default(),
            &ask_config(&cli),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("without any cards"));
    }

    #[test]
    fn local_file_source_is_embedded() {
        let dir = std::env::temp_dir().join(format!("alix-exam-src-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("notes.md");
        std::fs::write(&src, "the ground truth text").unwrap();
        let deck = Deck {
            path: dir.join("d.txt"),
            subject: "d.txt".to_string(),
            cards: Vec::new(),
            links: Vec::new(),
            requires: Vec::new(),
            sources: vec!["notes.md".to_string()],
            settings: Default::default(),
            title: None,
            trace: None,
        };
        let section = source_section(&deck.sources, deck.path.parent()).unwrap();
        assert!(section.contains("the ground truth text"));
        assert!(section.contains("notes.md"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Polls a sitting until its in-flight background call lands (or times
    /// out).
    fn drain(s: &mut Sitting, store: &mut Store) {
        for _ in 0..500 {
            if s.poll(store, 0) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("exam background call did not complete");
    }

    /// A fake CLI that answers the three exam calls by branching on the
    /// prompt's JSON-shape marker (`"grades"` / `"questions"`), else emits
    /// a deck.
    fn branching_cli(dir: &std::path::Path, grades: &str) -> std::path::PathBuf {
        let body = format!(
            "input=$(cat)\n\
             case \"$input\" in\n\
             *'\"grades\"'*) printf '%s' '{grades}' ;;\n\
             *'\"questions\"'*) printf '%s' '{{\"questions\":[{{\"prompt\":\"Q1\",\"points\":[\"p1\"]}},{{\"prompt\":\"Q2\",\"points\":[\"p2\"]}}]}}' ;;\n\
             *) printf '# Why does X?\\n%% mode: explain\\n\\tpoint one\\n' ;;\n\
             esac"
        );
        fake_cli(dir, &body)
    }

    fn sourced_deck(dir: &std::path::Path) -> Deck {
        let path = dir.join("d.txt");
        std::fs::write(&path, "% source: https://x\n# c\n\ta\n").unwrap();
        Deck::load(&path).unwrap()
    }

    #[test]
    fn sitting_drives_generate_answer_grade_remediate_on_fail() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        // One fail, one pass -> overall fail (default threshold = all pass).
        let cli = branching_cli(
            dir.path(),
            "{\"grades\":[{\"verdict\":\"fail\",\"feedback\":\"no\",\"missed\":[\"the move rule\"]},\
             {\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]}]}",
        );
        let deck = sourced_deck(dir.path());
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let mut s = Sitting::start(
            &deck,
            Strictness::Balanced,
            ExamConfig::default(),
            ask_config(&cli),
        );
        assert_eq!(&Phase::Generating, s.phase());
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Answering, s.phase());
        assert_eq!(2, s.total());

        // Navigation + per-question answers.
        assert!(!s.on_last());
        s.set_answer("a1".to_string());
        s.next();
        assert!(s.on_last());
        s.set_answer("a2".to_string());
        s.prev();
        assert_eq!("a1", s.answer());
        s.next();

        s.submit();
        assert_eq!(&Phase::Grading, s.phase());
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Results, s.phase());
        assert!(!s.result().unwrap().passed);
        assert_eq!(vec!["the move rule"], s.gaps());
        assert!(!store.deck_mastered("d.txt")); // failed -> not mastered

        assert!(s.can_remediate());
        let snapshot = std::fs::read(dir.path().join("d.txt")).unwrap();
        s.remediate();
        assert_eq!(&Phase::Remediating, s.phase());
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Remediated, s.phase());
        // The remediation card became a virtual card in the store — the deck
        // file itself stays byte-unchanged.
        assert_eq!(snapshot, std::fs::read(dir.path().join("d.txt")).unwrap());
        let virtuals = store.virtual_cards_for("d.txt");
        assert_eq!(1, virtuals.len());
        assert_eq!(VirtualKind::Remediation, virtuals[0].kind);
        assert_eq!(Some(Mode::Explain), virtuals[0].content.mode);
        assert_eq!(vec!["point one".to_string()], virtuals[0].content.back);
    }

    #[test]
    fn sitting_pass_marks_mastered() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = branching_cli(
            dir.path(),
            "{\"grades\":[{\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]},\
             {\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]}]}",
        );
        let deck = sourced_deck(dir.path());
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let mut s = Sitting::start(
            &deck,
            Strictness::Strict,
            ExamConfig::default(),
            ask_config(&cli),
        );
        drain(&mut s, &mut store);
        s.set_answer("a".to_string());
        s.next();
        s.set_answer("b".to_string());
        s.submit();
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Results, s.phase());
        assert!(s.result().unwrap().passed);
        assert!(store.deck_mastered("d.txt")); // pass -> mastered + saved
        assert!(!s.can_remediate());
    }

    // ── Remediation writes virtual cards (B.3) ──────────────────────────────

    #[test]
    fn a_failed_exam_creates_virtual_cards_and_leaves_the_deck_file_unchanged() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = branching_cli(
            dir.path(),
            "{\"grades\":[{\"verdict\":\"fail\",\"feedback\":\"no\",\"missed\":[\"the move rule\"]},\
             {\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]}]}",
        );
        let deck = sourced_deck(dir.path());
        let deck_path = dir.path().join("d.txt");
        let before = std::fs::read(&deck_path).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let mut s = Sitting::start(
            &deck,
            Strictness::Balanced,
            ExamConfig::default(),
            ask_config(&cli),
        );
        drain(&mut s, &mut store);
        s.set_answer("a1".to_string());
        s.next();
        s.set_answer("a2".to_string());
        s.submit();
        drain(&mut s, &mut store);
        assert!(s.can_remediate());

        s.remediate();
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Remediated, s.phase());

        let after = std::fs::read(&deck_path).unwrap();
        assert_eq!(before, after, "remediation must never touch the deck file");

        let virtuals = store.virtual_cards_for("d.txt");
        assert_eq!(1, virtuals.len());
        assert_eq!(VirtualKind::Remediation, virtuals[0].kind);
        assert_eq!("d.txt", virtuals[0].parent);
    }

    #[test]
    fn regenerating_the_same_remediation_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let text = "# Why does X?\n% mode: explain\n\tpoint one\n";

        let n1 = store_remediation_cards(&mut store, "d.txt", text, 1_000).unwrap();
        assert_eq!(1, n1);
        assert_eq!(1, store.virtual_cards_for("d.txt").len());

        let n2 = store_remediation_cards(&mut store, "d.txt", text, 2_000).unwrap();
        assert_eq!(0, n2, "an active dupe is left alone, not recreated");
        assert_eq!(1, store.virtual_cards_for("d.txt").len());
    }

    #[test]
    fn multi_hole_cloze_with_repeated_answer_keeps_all_subcards() {
        // Both holes hold the same token ("be"), so the sub-cards share
        // `front` and `back` — only `context` (which hole is blanked)
        // distinguishes them. The discriminator must fold context in, or the
        // second sub-card collides with the first and is silently dropped.
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let text = "#? Complete the quote\n\tTo {{be}} or not to {{be}}\n";

        let n = store_remediation_cards(&mut store, "d.txt", text, 1_000).unwrap();
        assert_eq!(2, n, "both cloze sub-cards should be created, not deduped");
        assert_eq!(2, store.virtual_cards_for("d.txt").len());
    }

    #[test]
    fn reviving_replaces_an_archived_remediation_card() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let text = "# Why does X?\n% mode: explain\n\tpoint one\n";

        store_remediation_cards(&mut store, "d.txt", text, 1_000).unwrap();
        let id = store.virtual_cards_for("d.txt")[0].id.clone();
        {
            let vc = store.get_virtual_mut(&id).unwrap();
            vc.retired = true;
            vc.state.total_reviews = 3;
        }

        let n = store_remediation_cards(&mut store, "d.txt", text, 5_000).unwrap();
        assert_eq!(1, n);
        assert_eq!(1, store.virtual_cards_for("d.txt").len());
        let vc = store.get_virtual(&id).unwrap();
        assert!(!vc.retired);
        assert_eq!(0, vc.state.total_reviews, "revive resets to a fresh state");
    }

    #[test]
    fn a_re_pass_does_not_retire_the_remediation_batch() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli_dir1 = tempfile::tempdir().unwrap();
        let cli1 = branching_cli(
            cli_dir1.path(),
            "{\"grades\":[{\"verdict\":\"fail\",\"feedback\":\"no\",\"missed\":[\"the move rule\"]},\
             {\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]}]}",
        );
        let deck = sourced_deck(dir.path());
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        // Fail then remediate: the batch exists.
        let mut s = Sitting::start(
            &deck,
            Strictness::Balanced,
            ExamConfig::default(),
            ask_config(&cli1),
        );
        drain(&mut s, &mut store);
        s.set_answer("a1".to_string());
        s.next();
        s.set_answer("a2".to_string());
        s.submit();
        drain(&mut s, &mut store);
        assert!(!s.result().unwrap().passed);
        s.remediate();
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Remediated, s.phase());
        assert!(!store.virtual_cards_for("d.txt").is_empty());

        // Re-sit and pass this time.
        let cli_dir2 = tempfile::tempdir().unwrap();
        let cli2 = branching_cli(
            cli_dir2.path(),
            "{\"grades\":[{\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]},\
             {\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]}]}",
        );
        let mut s2 = Sitting::start(
            &deck,
            Strictness::Balanced,
            ExamConfig::default(),
            ask_config(&cli2),
        );
        drain(&mut s2, &mut store);
        s2.set_answer("a".to_string());
        s2.next();
        s2.set_answer("b".to_string());
        s2.submit();
        drain(&mut s2, &mut store);
        assert!(s2.result().unwrap().passed);
        assert!(store.deck_mastered("d.txt"));

        for vc in store.virtual_cards_for("d.txt") {
            assert!(!vc.retired, "a passing re-sit must not retire remediation cards");
        }
    }

    #[test]
    fn remediation_card_mode_is_carried() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let text = "# Why does X?\n% mode: explain\n\tpoint one\n\n# fact card\n\tplain answer\n";

        store_remediation_cards(&mut store, "d.txt", text, 1_000).unwrap();
        let virtuals = store.virtual_cards_for("d.txt");
        let explain = virtuals
            .iter()
            .find(|c| c.content.front == "Why does X?")
            .unwrap();
        let plain = virtuals
            .iter()
            .find(|c| c.content.front == "fact card")
            .unwrap();
        assert_eq!(Some(Mode::Explain), explain.content.mode);
        assert_eq!(None, plain.content.mode);
    }

    // ── Trace exam (the compression) ────────────────────────────────────────

    #[test]
    fn grade_compression_prompt_carries_path_points_and_answer() {
        let p = grade_compression_prompt(
            "how a keypress becomes a saved grade",
            &[
                "the keypress posts only the grade".to_string(),
                "the server reschedules the card".to_string(),
            ],
            "you press a key and it saves",
            Strictness::Balanced,
        );
        assert!(p.contains("how a keypress becomes a saved grade"));
        assert!(p.contains("posts only the grade"));
        assert!(p.contains("you press a key and it saves"));
        assert!(p.contains("RE-DERIVE"));
        assert!(p.contains("\"verdict\""));
    }

    #[test]
    fn grade_compression_prompt_marks_an_empty_answer() {
        let p = grade_compression_prompt("d", &["p".to_string()], "   ", Strictness::Strict);
        assert!(p.contains("(no answer given)"));
    }

    #[test]
    fn compression_criteria_vary_by_strictness() {
        assert!(compression_strictness_criteria(Strictness::Strict).contains("every load-bearing"));
        assert!(compression_strictness_criteria(Strictness::Balanced).contains("RE-DERIVE"));
        assert!(compression_strictness_criteria(Strictness::Lenient).contains("generously"));
    }

    #[test]
    fn grade_compression_end_to_end_pass() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(
            dir.path(),
            "{\"verdict\":\"pass\",\"feedback\":\"passed the chain\",\"missed\":[]}",
        );
        let result = grade_compression(
            "how X becomes Y",
            &["step one".to_string()],
            "my retrace",
            Strictness::Balanced,
            &ExamConfig::default(),
            &ask_config(&cli),
        )
        .unwrap();
        assert!(result.passed);
        assert_eq!(Verdict::Pass, result.grades[0].verdict);
    }

    fn trace_sitting(cli: &std::path::Path) -> Sitting {
        Sitting::start_trace(
            "how a moves".to_string(),
            vec!["it advances".to_string()],
            "t.txt".to_string(),
            Strictness::Balanced,
            ExamConfig::default(),
            ask_config(cli),
        )
    }

    #[test]
    fn trace_sitting_passes_and_masters() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(
            dir.path(),
            "{\"verdict\":\"pass\",\"feedback\":\"good\",\"missed\":[]}",
        );
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let mut s = trace_sitting(&cli);
        assert_eq!(SittingKind::Trace, s.kind());
        assert_eq!(&Phase::Answering, s.phase()); // no generation step
        assert_eq!(1, s.total());
        s.set_answer("my retrace of the path".to_string());
        s.submit();
        assert_eq!(&Phase::Grading, s.phase());
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Results, s.phase());
        assert!(s.result().unwrap().passed);
        assert!(store.deck_mastered("t.txt")); // pass -> mastered
        assert!(!s.can_remediate()); // never for a trace
    }

    #[test]
    fn trace_sitting_fail_starts_cooldown_without_mastering() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(
            dir.path(),
            "{\"verdict\":\"fail\",\"feedback\":\"missed the return path\",\"missed\":[\"the return\"]}",
        );
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let mut s = trace_sitting(&cli);
        s.set_answer("wrong".to_string());
        s.submit();
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Results, s.phase());
        assert!(!s.result().unwrap().passed);
        assert!(!store.deck_mastered("t.txt"));
        assert!(store.exam_failed_at("t.txt").is_some()); // re-sit cooldown started
        assert!(!s.can_remediate());
    }

    #[test]
    fn cooldown_remaining_reflects_failed_time_and_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        // Never failed -> no cooldown.
        assert_eq!(None, cooldown_remaining_ms(&store, "t.txt", 3600, 0));
        store.set_exam_failed("t.txt", 1_000);
        // 1h cooldown, 30s after the fail -> the rest of the hour remains.
        let now = 1_000 + 30_000;
        assert_eq!(
            Some(3_600_000 - 30_000),
            cooldown_remaining_ms(&store, "t.txt", 3600, now)
        );
        // Once the hour elapses -> None.
        assert_eq!(
            None,
            cooldown_remaining_ms(&store, "t.txt", 3600, 1_000 + 3_600_001)
        );
        // Disabled (0) -> None even right after a fail.
        assert_eq!(None, cooldown_remaining_ms(&store, "t.txt", 0, now));
    }
}
