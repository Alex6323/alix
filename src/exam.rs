use std::{
    collections::HashSet,
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

const MAX_SOURCE_BYTES: usize = 100_000;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ExamQuestion {
    pub prompt: String,
    pub points: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Partial,
    Fail,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Partial => "PARTIAL",
            Verdict::Fail => "FAIL",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct AnswerGrade {
    pub verdict: Verdict,
    pub feedback: String,
    #[serde(default)]
    pub missed: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExamResult {
    pub passed: bool,
    pub grades: Vec<AnswerGrade>,
}

impl ExamResult {
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

pub fn generate_questions(
    deck: &Deck,
    cfg: &ExamConfig,
    ask_cfg: &AskConfig,
) -> Result<Vec<ExamQuestion>> {
    if deck.sources.is_empty() {
        bail!("the deck declares no `source:` to examine against");
    }
    generate_questions_from(&deck.sources, deck.path.parent(), cfg, ask_cfg)
}

pub fn ensure_backend_can_examine(deck: &Deck, ask_cfg: &AskConfig) -> Result<()> {
    for source in &deck.sources {
        crate::backend::ensure_source_reachable(ask_cfg, deck::is_url(source))?;
    }
    Ok(())
}

fn generate_questions_from(
    sources: &[String],
    base: Option<&Path>,
    cfg: &ExamConfig,
    ask_cfg: &AskConfig,
) -> Result<Vec<ExamQuestion>> {
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

pub fn remediation_cards(gaps: &[String], cfg: &ExamConfig, ask_cfg: &AskConfig) -> Result<String> {
    if gaps.is_empty() {
        bail!("no gaps to remediate");
    }
    let prompt = remediation_prompt(gaps);
    let mut cfg_run = run_config(cfg, ask_cfg);
    cfg_run.allowed_tools.clear(); // no web access needed, so it can't wander off
    let raw = ask::run(&cfg_run, &prompt, &[])?;
    let cards = clean_deck_output(&raw);
    if !cards.lines().any(|l| l.starts_with("## ")) {
        bail!("the model replied without any cards — try remediating again");
    }
    Ok(cards)
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Generating,
    Answering,
    Grading,
    Results,
    Remediating,
    Remediated,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Passed,
    TraceFailed,
    RemediationCards(String),
}

enum Pending {
    Questions(Receiver<Result<Vec<ExamQuestion>, String>>),
    Grade(Receiver<Result<ExamResult, String>>),
    Remediation(Receiver<Result<String, String>>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SittingKind {
    Source,
    Trace,
}

pub struct Sitting {
    kind: SittingKind,
    subject: String,
    deck_fingerprints: HashSet<u64>,
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
    pending_since: Option<u64>,
    remediated_count: Option<usize>,
}

impl Sitting {
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
            deck_fingerprints: deck.cards.iter().map(|c| c.content_fingerprint).collect(),
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
            remediated_count: None,
        }
    }

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
            deck_fingerprints: HashSet::new(),
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
            remediated_count: None,
        }
    }

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
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn thinking(&self) -> bool {
        self.pending.is_some()
    }
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
    pub fn question(&self) -> Option<&ExamQuestion> {
        self.questions.get(self.current)
    }
    pub fn answer(&self) -> &str {
        self.answers
            .get(self.current)
            .map(String::as_str)
            .unwrap_or("")
    }
    pub fn on_last(&self) -> bool {
        !self.questions.is_empty() && self.current + 1 == self.questions.len()
    }

    pub fn set_answer(&mut self, text: String) {
        if self.phase == Phase::Answering
            && let Some(slot) = self.answers.get_mut(self.current)
        {
            *slot = text;
        }
    }

    pub fn set_answers(&mut self, answers: Vec<String>) -> bool {
        if self.phase == Phase::Answering && answers.len() == self.questions.len() {
            self.answers = answers;
            true
        } else {
            false
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

    pub fn gaps(&self) -> Vec<String> {
        self.result
            .as_ref()
            .map(ExamResult::gaps)
            .unwrap_or_default()
    }

    pub fn can_remediate(&self) -> bool {
        self.kind == SittingKind::Source
            && self.phase == Phase::Results
            && self.result.as_ref().is_some_and(|r| !r.passed)
            && !self.gaps().is_empty()
    }

    pub fn remediated_count(&self) -> Option<usize> {
        self.remediated_count
    }

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

    pub fn advance(&mut self, _now_ms: u64) -> Option<Effect> {
        let reply = match &self.pending {
            None => return None,
            Some(Pending::Questions(rx)) => match rx.try_recv() {
                Ok(r) => Reply::Questions(r),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => Reply::Questions(Err(thread_gone())),
            },
            Some(Pending::Grade(rx)) => match rx.try_recv() {
                Ok(r) => Reply::Grade(r),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => Reply::Grade(Err(thread_gone())),
            },
            Some(Pending::Remediation(rx)) => match rx.try_recv() {
                Ok(r) => Reply::Remediation(r),
                Err(TryRecvError::Empty) => return None,
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
                None
            }
            Reply::Questions(Err(e)) => {
                self.error = Some(e);
                None
            }
            Reply::Grade(Ok(result)) => {
                let effect = if result.passed {
                    Some(Effect::Passed)
                } else if self.kind == SittingKind::Trace {
                    Some(Effect::TraceFailed)
                } else {
                    None
                };
                self.result = Some(result);
                self.phase = Phase::Results;
                effect
            }
            Reply::Grade(Err(e)) => {
                self.error = Some(e);
                self.phase = Phase::Answering;
                None
            }
            Reply::Remediation(Ok(cards)) => {
                self.phase = Phase::Remediated;
                Some(Effect::RemediationCards(cards))
            }
            Reply::Remediation(Err(e)) => {
                self.error = Some(e);
                self.phase = Phase::Results;
                None
            }
        }
    }

    pub fn poll(&mut self, store: &mut Store, now_ms: u64, retire_after_days: Option<u32>) -> bool {
        let was_pending = self.pending.is_some();
        let effect = self.advance(now_ms);
        let advanced = was_pending && self.pending.is_none();
        match effect {
            Some(Effect::Passed) => {
                store.set_deck_mastered(&self.subject, now_ms);
                let _ = store.save();
            }
            Some(Effect::TraceFailed) => {
                store.set_exam_failed(&self.subject, now_ms);
                let _ = store.save();
            }
            Some(Effect::RemediationCards(cards)) => {
                match crate::store::store_remediation_cards(
                    store,
                    &self.subject,
                    &self.deck_fingerprints,
                    &cards,
                    now_ms,
                    retire_after_days,
                ) {
                    Ok(n) => self.remediated_count = Some(n),
                    Err(e) => {
                        self.error = Some(format!("{e}"));
                        self.phase = Phase::Results;
                    }
                }
            }
            None => {}
        }
        advanced
    }
}

enum Reply {
    Questions(Result<Vec<ExamQuestion>, String>),
    Grade(Result<ExamResult, String>),
    Remediation(Result<String, String>),
}

fn thread_gone() -> String {
    "the exam helper exited unexpectedly".to_string()
}

pub use crate::store::cooldown_remaining_ms;

fn passed(grades: &[AnswerGrade], threshold: f64) -> bool {
    if grades.is_empty() {
        return false;
    }
    let passes = grades.iter().filter(|g| g.verdict == Verdict::Pass).count();
    (passes as f64) / (grades.len() as f64) >= threshold
}

fn run_config(cfg: &ExamConfig, ask_cfg: &AskConfig) -> AskConfig {
    AskConfig {
        model: cfg.model.clone().or_else(|| ask_cfg.model.clone()),
        timeout_secs: cfg.timeout_secs,
        cwd: None,
        source_access: false,
        ..ask_cfg.clone()
    }
}

fn source_section(sources: &[String], base: Option<&Path>) -> Result<String> {
    let mut urls = Vec::new();
    let mut files = Vec::new();
    for src in sources {
        if deck::is_url(src) {
            urls.push(src.clone());
            continue;
        }
        // A value may join several files with " + "; skip any that can't be read rather than
        // failing.
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
                    "warning: skipping unreadable `source:` {}: {e}",
                    path.display()
                ),
            }
        }
    }
    if urls.is_empty() && files.is_empty() {
        bail!("none of the deck's `source:` paths could be read to examine against");
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
Grade generously: the goal is only to catch real misunderstandings. Give the \
student the benefit of the doubt on anything plausibly correct or partially \
stated. An answer covering only some of the key points still passes when what \
it says is correct; incompleteness alone is never \"partial\" here. Reserve \
\"partial\" for an actual error. Put a point in `missed` ONLY if the answer is \
clearly wrong about it or the question was essentially not answered.\n\
- \"pass\": broadly correct, even if thin or incomplete.\n\
- \"partial\": partly right but with a clear error.\n\
- \"fail\": wrong or essentially unanswered."
        }
    }
}

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
         cheap recall card. Prefer a cloze: a `## ` front with a short \
         instruction, then an answer line that states the fact with each hidden \
         span wrapped as `\\blank{...}` (braces outside the marker are literal). \
         If there is no natural word to blank out, use a plain `## ` card with \
         the answer on the line below instead.\n\
         - A missed CONCEPT, MECHANISM or CONNECTION (a \"why\", \"how\" or \"what \
         happens if\") -> an understanding card: a `## ` open prompt with the \
         key points a good answer covers, one per line below it. This forces the \
         student to re-derive the idea, which is what the exam re-tests. When \
         unsure, prefer the understanding card.\n\n\
         CONCEPTS THE STUDENT MISSED:\n",
    );
    for gap in gaps {
        prompt.push_str("  - ");
        prompt.push_str(gap);
        prompt.push('\n');
    }
    prompt.push_str(
        "\nFORMAT — a Markdown deck, cards one after another. A card is a `## ` \
         front at column 0 (never indented); its answer is the plain \
         (unindented) line(s) below it; a `> ` line after them adds an optional \
         note (a caveat, example or why it matters). No frontmatter, no \
         headings other than the `## ` fronts. One example of each card type:\n\
         ## Recall how a String is laid out in memory.\n\
         A String stores a \\blank{pointer}, \\blank{length} and \
         \\blank{capacity} on the stack, and its bytes live on the \\blank{heap}.\n\
         ## What does `drop` do for a String, and when?\n\
         It returns the String's heap buffer to the allocator, at the end of the \
         owning scope.\n\
         ## Why does moving a String invalidate the original binding?\n\
         Both bindings would otherwise point at the same heap buffer.\n\
         Dropping both would free it twice (a double free).\n\
         So Rust invalidates the source instead of allowing two owners.\n\
         > A move, not a shallow copy you can keep using.\n\n\
         Before finishing, re-read your cards as a set and merge any two that test \
         the same idea, so every card is distinct.\n\
         Output ONLY the deck text — no markdown code fences, no preamble, no \
         closing remarks.",
    );
    prompt
}

fn extract_json(raw: &str) -> &str {
    match (raw.find('{'), raw.rfind('}')) {
        (Some(start), Some(end)) if end > start => &raw[start..=end],
        _ => raw.trim(),
    }
}

fn parse_json<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T> {
    let json = extract_json(raw);
    serde_json::from_str(json)
        .with_context(|| format!("the model did not return valid JSON:\n{json}"))
}

fn clean_deck_output(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let Some(start) = lines.iter().position(|l| l.starts_with("## ")) else {
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
            path: std::path::PathBuf::from("/tmp/d.md"),
            subject: "d.md".to_string(),
            deck_token: None,
            cards: Vec::new(),
            links: Vec::new(),
            requires: Vec::new(),
            sources: srcs.iter().map(|s| s.to_string()).collect(),
            settings: Default::default(),
            title: None,
            preamble: None,
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
        assert!(strict.contains("COMPLETENESS"));
        assert!(strict.contains("EVERY key point"));
        assert!(balanced.contains("UNDERSTANDING, not completeness"));
        assert!(!balanced.contains("COMPLETENESS — treat the rubric"));
        assert!(lenient.contains("benefit of the doubt"));
        assert!(lenient.contains("incompleteness alone is never"));
        assert!(!strict.contains("incompleteness alone is never"));
        assert!(!balanced.contains("incompleteness alone is never"));
    }

    #[test]
    fn remediation_prompt_asks_for_understanding_cards() {
        let p = remediation_prompt(&["the aliasing rule".to_string()]);
        assert!(p.contains("the aliasing rule"));
        assert!(p.contains("understanding card"));
        assert!(p.contains("Output ONLY the deck text"));
    }

    #[test]
    fn remediation_prompt_picks_card_type_per_gap() {
        let p = remediation_prompt(&["x".to_string()]);
        assert!(p.contains("missed FACT or TERM"));
        assert!(p.contains("missed CONCEPT"));
        assert!(p.contains("\\blank{...}"));
        assert!(p.contains("understanding card"));
        assert!(!p.contains("indented answer"));
        assert!(p.contains("## "));
    }

    #[test]
    fn remediation_prompt_asks_to_dedup_overlapping_gaps() {
        let p = remediation_prompt(&[
            "String stores a pointer, length and capacity on the stack".to_string(),
            "A String keeps its bytes on the heap and a pointer on the stack".to_string(),
        ]);
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
        let raw = "```text\n## Q\nA\n```";
        assert_eq!("## Q\nA", clean_deck_output(raw));
    }

    use crate::{
        parser,
        session::is_retired_id,
        store::VirtualKind,
        testutil::{ask_config, exec_lock, fake_cli, fake_reply},
    };

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
        assert!(format!("{err:#}").contains("no `source:`"));
    }

    #[test]
    fn generate_questions_from_rejects_url_source_on_fetch_incapable_backend() {
        use crate::config::BackendKind;
        let ask_cfg = AskConfig {
            backend: BackendKind::Codex,
            command: "/dev/null".to_string(),
            timeout_secs: 5,
            ..AskConfig::default()
        };
        // No exec_lock: the gate fires before any subprocess is forked.
        let rx = spawn_questions(
            vec!["https://example.org/doc".to_string()],
            None,
            ExamConfig::default(),
            ask_cfg,
        );
        let err = rx.recv().unwrap().unwrap_err();
        assert!(
            err.contains("codex") && err.contains("can't fetch a url"),
            "expected capability-refusal message, got: {err}"
        );
    }

    #[test]
    fn ensure_backend_can_examine_rejects_url_source_on_fetch_incapable_backend() {
        use crate::config::BackendKind;
        let deck = deck_with_sources(&["https://example.org/doc"]);
        let ask_cfg = AskConfig {
            backend: BackendKind::Codex,
            command: "/dev/null".to_string(),
            timeout_secs: 5,
            ..AskConfig::default()
        };
        let err = ensure_backend_can_examine(&deck, &ask_cfg).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("codex") && msg.contains("can't fetch a url"),
            "expected capability-refusal message, got: {msg}"
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
        let cli = fake_reply(dir.path(), "```text\n## Why?\npoint\n```\n");
        let cards = remediation_cards(
            &["the gap".to_string()],
            &ExamConfig::default(),
            &ask_config(&cli),
        )
        .unwrap();
        assert_eq!("## Why?\npoint", cards);
    }

    #[test]
    fn remediation_cards_rejects_a_reply_without_cards() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
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
            path: dir.join("d.md"),
            subject: "d.md".to_string(),
            deck_token: None,
            cards: Vec::new(),
            links: Vec::new(),
            requires: Vec::new(),
            sources: vec!["notes.md".to_string()],
            settings: Default::default(),
            title: None,
            preamble: None,
            trace: None,
        };
        let section = source_section(&deck.sources, deck.path.parent()).unwrap();
        assert!(section.contains("the ground truth text"));
        assert!(section.contains("notes.md"));
        std::fs::remove_dir_all(&dir).ok();
    }

    fn drain(s: &mut Sitting, store: &mut Store) {
        for _ in 0..500 {
            if s.poll(store, 0, Some(crate::session::DEFAULT_RETIRE_AFTER_DAYS)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("exam background call did not complete");
    }

    fn advance_until_idle(s: &mut Sitting) -> Option<Effect> {
        for _ in 0..500 {
            let effect = s.advance(0);
            if !s.thinking() {
                return effect;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("exam background call did not complete");
    }

    fn branching_cli(dir: &std::path::Path, grades: &str) -> std::path::PathBuf {
        let body = format!(
            "input=$(cat)\n\
             case \"$input\" in\n\
             *'\"grades\"'*) printf '%s' '{grades}' ;;\n\
             *'\"questions\"'*) printf '%s' '{{\"questions\":[{{\"prompt\":\"Q1\",\"points\":[\"p1\"]}},{{\"prompt\":\"Q2\",\"points\":[\"p2\"]}}]}}' ;;\n\
             *) printf '## Why does X?\\npoint one\\n' ;;\n\
             esac"
        );
        fake_cli(dir, &body)
    }

    fn sourced_deck(dir: &std::path::Path) -> Deck {
        let path = dir.join("d.md");
        std::fs::write(
            &path,
            "---\nsource: https://x\n---\n## c <!-- id: qc -->\na\n",
        )
        .unwrap();
        Deck::load(&path).unwrap()
    }

    #[test]
    fn sitting_drives_generate_answer_grade_remediate_on_fail() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
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
        assert!(!store.deck_mastered("d.md"));

        assert!(s.can_remediate());
        let snapshot = std::fs::read(dir.path().join("d.md")).unwrap();
        s.remediate();
        assert_eq!(&Phase::Remediating, s.phase());
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Remediated, s.phase());
        assert_eq!(snapshot, std::fs::read(dir.path().join("d.md")).unwrap());
        let virtuals = store.virtual_cards_for("d.md");
        assert_eq!(1, virtuals.len());
        assert_eq!(VirtualKind::Remediation, virtuals[0].kind);
        let synth = parser::parse_str("d.md", &virtuals[0].text).unwrap();
        assert_eq!("Why does X?", synth[0].front);
        assert_eq!(vec!["point one".to_string()], synth[0].back);
    }

    #[test]
    fn exam_fail_reports_the_remediation_count() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
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
        drain(&mut s, &mut store);
        s.set_answer("a1".to_string());
        s.next();
        s.set_answer("a2".to_string());
        s.submit();
        drain(&mut s, &mut store);
        assert!(s.can_remediate());
        assert_eq!(None, s.remediated_count());

        s.remediate();
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Remediated, s.phase());

        let virtuals = store.virtual_cards_for("d.md");
        assert_eq!(1, virtuals.len());
        assert_eq!(Some(virtuals.len()), s.remediated_count());
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
        assert!(store.deck_mastered("d.md"));
        assert!(!s.can_remediate());
    }

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
        let deck_path = dir.path().join("d.md");
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

        let virtuals = store.virtual_cards_for("d.md");
        assert_eq!(1, virtuals.len());
        assert_eq!(VirtualKind::Remediation, virtuals[0].kind);
        assert_eq!("d.md", virtuals[0].parent);
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
        assert!(!store.virtual_cards_for("d.md").is_empty());

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
        assert!(store.deck_mastered("d.md"));

        for vc in store.virtual_cards_for("d.md") {
            assert!(
                !is_retired_id(
                    &vc.id,
                    &store,
                    Some(crate::session::DEFAULT_RETIRE_AFTER_DAYS)
                ),
                "a passing re-sit must not retire remediation cards"
            );
        }
    }

    #[test]
    fn advance_surfaces_remediation_text_without_a_store() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
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
        drain(&mut s, &mut store);
        s.set_answer("a1".to_string());
        s.next();
        s.set_answer("a2".to_string());
        s.submit();
        drain(&mut s, &mut store);
        assert!(s.can_remediate());
        s.remediate();
        assert_eq!(&Phase::Remediating, s.phase());

        let effect = advance_until_idle(&mut s);
        let Some(Effect::RemediationCards(text)) = effect else {
            panic!("expected Some(Effect::RemediationCards(_)), got {effect:?}");
        };
        let synth = parser::parse_str("d.md", &text).unwrap();
        assert_eq!("Why does X?", synth[0].front);
        assert_eq!(vec!["point one".to_string()], synth[0].back);
        assert_eq!(&Phase::Remediated, s.phase());
        assert_eq!(None, s.remediated_count());
    }

    #[test]
    fn advance_keeps_grade_fail_storeless() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = branching_cli(
            dir.path(),
            "{\"grades\":[{\"verdict\":\"fail\",\"feedback\":\"no\",\"missed\":[\"the move rule\"]},\
             {\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]}]}",
        );
        let deck = sourced_deck(dir.path());

        let mut s = Sitting::start(
            &deck,
            Strictness::Balanced,
            ExamConfig::default(),
            ask_config(&cli),
        );
        assert_eq!(None, advance_until_idle(&mut s));
        assert_eq!(&Phase::Answering, s.phase());
        s.set_answer("a1".to_string());
        s.next();
        s.set_answer("a2".to_string());
        s.submit();

        let effect = advance_until_idle(&mut s);
        assert_eq!(None, effect);
        assert_eq!(&Phase::Results, s.phase());
        assert!(!s.result().unwrap().passed);
    }

    #[test]
    fn set_answers_rejects_wrong_arity_and_wrong_phase() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = branching_cli(
            dir.path(),
            "{\"grades\":[{\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]},\
             {\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]}]}",
        );
        let deck = sourced_deck(dir.path());

        let mut s = Sitting::start(
            &deck,
            Strictness::Balanced,
            ExamConfig::default(),
            ask_config(&cli),
        );
        assert_eq!(&Phase::Generating, s.phase());
        assert!(!s.set_answers(Vec::new()));
        assert!(s.answers().is_empty());

        advance_until_idle(&mut s);
        assert_eq!(&Phase::Answering, s.phase());
        assert_eq!(2, s.total());

        assert!(!s.set_answers(vec!["only one".to_string()]));
        assert_eq!(vec!["".to_string(), "".to_string()], s.answers());

        assert!(s.set_answers(vec!["a1".to_string(), "a2".to_string()]));
        assert_eq!(vec!["a1".to_string(), "a2".to_string()], s.answers());
    }

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
        assert_eq!(&Phase::Answering, s.phase());
        assert_eq!(1, s.total());
        s.set_answer("my retrace of the path".to_string());
        s.submit();
        assert_eq!(&Phase::Grading, s.phase());
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Results, s.phase());
        assert!(s.result().unwrap().passed);
        assert!(store.deck_mastered("t.txt"));
        assert!(!s.can_remediate());
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
        assert!(store.exam_failed_at("t.txt").is_some());
        assert!(!s.can_remediate());
    }
}
