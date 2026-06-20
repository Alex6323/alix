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
//! consumer (`flash exam`) drives the terminal Q&A; a web exam surface can
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
    store::Store,
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
    generate_questions_from(&deck.sources, deck.path.parent(), cfg, ask_cfg)
}

/// Owned-input core of [`generate_questions`]: takes the source list and the
/// base directory directly (not a `&Deck`), so the background
/// [`spawn_questions`] can run it on a thread without borrowing a deck.
fn generate_questions_from(
    sources: &[String],
    base: Option<&Path>,
    cfg: &ExamConfig,
    ask_cfg: &AskConfig,
) -> Result<Vec<ExamQuestion>> {
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

/// Turns the missed `gaps` into cards — a cloze/plain card for a missed fact, a
/// `% mode: explain` card for a missed concept — and returns the cleaned
/// deck-format text, ready to append to the deck file.
pub fn remediation_cards(gaps: &[String], cfg: &ExamConfig, ask_cfg: &AskConfig) -> Result<String> {
    if gaps.is_empty() {
        bail!("no gaps to remediate");
    }
    let prompt = remediation_prompt(gaps);
    let raw = ask::run(&run_config(cfg, ask_cfg), &prompt, &[])?;
    let cards = clean_deck_output(&raw);
    if cards.trim().is_empty() {
        bail!("the model returned no remediation cards");
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
    /// Generating + appending remediation cards (background call in flight).
    Remediating,
    /// Remediation cards were appended — re-drill the deck and re-sit.
    Remediated,
}

/// The in-flight background call for a [`Sitting`].
enum Pending {
    Questions(Receiver<Result<Vec<ExamQuestion>, String>>),
    Grade(Receiver<Result<ExamResult, String>>),
    Remediation(Receiver<Result<String, String>>),
}

/// One in-progress exam sitting — a frontend-agnostic state machine shared by
/// the web server (`serve.rs`) and the TUI (`tui.rs`); the CLI keeps its own
/// linear flow. It owns the exam state and the in-flight background call,
/// spawns each engine step, and on [`poll`](Sitting::poll) transitions and
/// applies the side effects (persist "mastered" on a pass, append remediation
/// cards on confirm). Frontends drive entry/navigation/submit/remediate, call
/// `poll` each tick, and render off `phase()`.
pub struct Sitting {
    subject: String,
    deck_path: PathBuf,
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
            subject: deck.subject.clone(),
            deck_path: deck.path.clone(),
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
        }
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
    /// [`Phase::Grading`]).
    pub fn submit(&mut self) {
        if self.phase != Phase::Answering {
            return;
        }
        self.error = None;
        self.pending = Some(Pending::Grade(spawn_grade(
            self.questions.clone(),
            self.answers.clone(),
            self.strictness,
            self.cfg.clone(),
            self.ask_cfg.clone(),
        )));
        self.phase = Phase::Grading;
    }

    /// The missed gaps from the result (empty until graded).
    pub fn gaps(&self) -> Vec<String> {
        self.result
            .as_ref()
            .map(ExamResult::gaps)
            .unwrap_or_default()
    }

    /// Whether remediation is offerable (failed result with gaps to fix).
    pub fn can_remediate(&self) -> bool {
        self.phase == Phase::Results
            && self.result.as_ref().is_some_and(|r| !r.passed)
            && !self.gaps().is_empty()
    }

    /// Generates remediation cards for the gaps and appends them to the deck
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
        self.phase = Phase::Remediating;
    }

    /// Drains a finished background call and advances the phase, applying side
    /// effects: on a passing grade, persist "mastered" and save `store`; on
    /// remediation, append the cards to the deck file. Returns `true` when the
    /// phase advanced.
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
                }
                self.result = Some(result);
                self.phase = Phase::Results;
            }
            // Grading failed: back to answering so the student can resubmit.
            Reply::Grade(Err(e)) => {
                self.error = Some(e);
                self.phase = Phase::Answering;
            }
            Reply::Remediation(Ok(cards)) => match deck::append_cards(&self.deck_path, &cards) {
                Ok(()) => self.phase = Phase::Remediated,
                Err(e) => {
                    self.error = Some(format!("{e}"));
                    self.phase = Phase::Results;
                }
            },
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
        command: ask_cfg.command.clone(),
        permission_mode: ask_cfg.permission_mode.clone(),
        allowed_tools: ask_cfg.allowed_tools.clone(),
        model: cfg.model.clone().or_else(|| ask_cfg.model.clone()),
        effort: ask_cfg.effort.clone(),
        timeout_secs: cfg.timeout_secs,
        cwd: None,
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
        } else {
            let path = match base {
                Some(dir) => dir.join(src),
                None => std::path::PathBuf::from(src),
            };
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("cannot read source file {}", path.display()))?;
            files.push((src.clone(), truncate(&text)));
        }
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
/// a marker when it had to cut.
fn truncate(text: &str) -> String {
    if text.len() <= MAX_SOURCE_BYTES {
        return text.to_string();
    }
    let mut end = MAX_SOURCE_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[... source truncated ...]", &text[..end])
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

    /// Serializes tests that write + exec scripts (a concurrent writer's fd
    /// would make exec fail with ETXTBSY), like the ask tests.
    static EXEC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Writes a fake `claude` CLI that emits `body` and returns its path.
    fn fake_cli(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-claude");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn ask_config(command: &std::path::Path) -> AskConfig {
        AskConfig {
            command: command.to_str().unwrap().to_string(),
            timeout_secs: 10,
            ..AskConfig::default()
        }
    }

    #[test]
    fn generate_questions_end_to_end() {
        let _lock = EXEC_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let json = "{\"questions\":[{\"prompt\":\"Why move?\",\
                    \"points\":[\"avoids double free\"]}]}";
        let cli = fake_cli(dir.path(), &format!("printf '%s' '{json}'"));
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
        let _lock = EXEC_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let json = "{\"grades\":[{\"verdict\":\"pass\",\"feedback\":\"good\",\"missed\":[]}]}";
        let cli = fake_cli(dir.path(), &format!("printf '%s' '{json}'"));
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
        let _lock = EXEC_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // A fenced deck; clean_deck_output must strip the fences.
        let cli = fake_cli(
            dir.path(),
            "printf '%s\\n' '```text' '# Why?' '% mode: explain' '  point' '```'",
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
    fn local_file_source_is_embedded() {
        let dir = std::env::temp_dir().join(format!("flash-exam-src-{}", std::process::id()));
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
        let _lock = EXEC_LOCK.lock().unwrap();
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
        s.remediate();
        assert_eq!(&Phase::Remediating, s.phase());
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Remediated, s.phase());
        // The remediation card was appended to the deck file.
        let text = std::fs::read_to_string(dir.path().join("d.txt")).unwrap();
        assert!(text.contains("% mode: explain"));
    }

    #[test]
    fn sitting_pass_marks_mastered() {
        let _lock = EXEC_LOCK.lock().unwrap();
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
}
