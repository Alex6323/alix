use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, TryRecvError, channel},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub use crate::exam_status::{
    ExamFilter, ExamScope, ExamSection, ExamStatus, SECTION_NUDGE_CARDS, SectionStatus,
    SectionTally, exam_sections, exam_status, oversized_sections,
};
use crate::{
    ask::{self, parse_json},
    config::{AskConfig, ExamConfig, Strictness},
    deck::{self, Deck, ExamLayers},
    store::Store,
    workspace,
};

const MAX_SOURCE_BYTES: usize = 100_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExamQuestion {
    pub prompt: String,
    pub points: Vec<String>,
    pub section: usize,
    pub shape: Shape,
    pub model: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    #[default]
    Instance,
    Boundary,
    Fix,
}

impl Shape {
    pub fn label(self) -> &'static str {
        match self {
            Shape::Instance => "instance",
            Shape::Boundary => "boundary",
            Shape::Fix => "fix",
        }
    }
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
    pub grades: Vec<AnswerGrade>,
    pub sections: Vec<SectionScore>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SectionScore {
    pub pass: u32,
    pub partial: u32,
    pub total: u32,
    pub passed: bool,
}

impl ExamResult {
    pub fn scored(questions: &[ExamQuestion], grades: Vec<AnswerGrade>, threshold: f64) -> Self {
        let count = questions.iter().map(|q| q.section + 1).max().unwrap_or(0);
        let mut sections = vec![SectionScore::default(); count];
        for (question, grade) in questions.iter().zip(&grades) {
            let score = &mut sections[question.section];
            score.total += 1;
            match grade.verdict {
                Verdict::Pass => score.pass += 1,
                Verdict::Partial => score.partial += 1,
                Verdict::Fail => {}
            }
        }
        for score in &mut sections {
            score.passed =
                score.total > 0 && f64::from(score.pass) / f64::from(score.total) >= threshold;
        }
        Self { grades, sections }
    }

    pub fn count(&self) -> SectionScore {
        let mut count = SectionScore::default();
        for score in &self.sections {
            count.pass += score.pass;
            count.partial += score.partial;
            count.total += score.total;
        }
        count.passed = count.total > 0 && count.pass == count.total;
        count
    }

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
    let sources = examination_sources(deck);
    if sources.is_empty() {
        bail!("the deck and its workspace declare no `exam:` to examine against");
    }
    if let Some(error) = source_boundary_error(deck) {
        bail!("{error}");
    }
    let root = workspace::content_root(&deck.path);
    generate_questions_from(&sources, Some(&root), &exam_sections(deck), cfg, ask_cfg)
}

pub fn open_for_exam(path: &Path) -> Result<Deck> {
    crate::assemble::stamp_for_session(path)?;
    let mut deck = Deck::load(path).with_context(|| format!("loading {}", path.display()))?;
    deck.cards.retain(|card| card.id().is_some());
    Ok(deck)
}

/// Both layers, told apart (ADR 0026): the deck's own sources are the primary
/// grounding, the workspace source is supporting context, never shadowed.
fn examination_sources(deck: &Deck) -> ExamLayers {
    deck.exam_layers()
}

fn source_boundary_error(deck: &Deck) -> Option<String> {
    (workspace::root_for_deck(&deck.path).is_some() && deck.deck_token.is_some())
        .then(|| crate::assets::validate_member(deck).err())
        .flatten()
        .map(|error| format!("{error:#}"))
}

pub fn ensure_backend_can_examine(deck: &Deck, ask_cfg: &AskConfig) -> Result<()> {
    let sources = examination_sources(deck);
    for source in sources.own.iter().chain(&sources.workspace) {
        crate::backend::ensure_source_reachable(ask_cfg, deck::is_url(source))?;
    }
    Ok(())
}

fn generate_questions_from(
    sources: &ExamLayers,
    base: Option<&Path>,
    sections: &[ExamSection],
    cfg: &ExamConfig,
    ask_cfg: &AskConfig,
) -> Result<Vec<ExamQuestion>> {
    for source in sources.own.iter().chain(&sources.workspace) {
        crate::backend::ensure_source_reachable(ask_cfg, deck::is_url(source))?;
    }
    if sections.is_empty() {
        bail!("the deck has no cards to examine");
    }
    let run = run_config(cfg, ask_cfg);
    let models = with_one_retry(
        &run,
        &models_prompt(sources, base, sections, cfg)?,
        |raw| checked_models(sections.len(), raw),
        "the listed models",
    )?;
    with_one_retry(
        &run,
        &questions_prompt(sources, base, sections, &models, cfg)?,
        |raw| checked_questions(&models, raw),
        "the generated questions",
    )
}

fn with_one_retry<T>(
    run: &AskConfig,
    prompt: &str,
    check: impl Fn(&str) -> std::result::Result<T, String>,
    what: &str,
) -> Result<T> {
    let reason = match check(&ask::run(run, prompt, &[])?) {
        Ok(value) => return Ok(value),
        Err(reason) => reason,
    };
    let retry = format!(
        "{prompt}\n\nYour previous answer was rejected: {reason}. Write the whole answer \
         again and follow every rule above."
    );
    match check(&ask::run(run, &retry, &[])?) {
        Ok(value) => Ok(value),
        Err(reason) => bail!("{what} were rejected twice: {reason}"),
    }
}

fn checked_models(
    section_count: usize,
    raw: &str,
) -> std::result::Result<Vec<Vec<String>>, String> {
    let dto: ModelsDto =
        parse_json(raw).map_err(|error| format!("it was not the JSON asked for ({error:#})"))?;
    let mut models: Vec<Vec<String>> = vec![Vec::new(); section_count];
    for listed in dto.sections {
        let Some(slot) = listed
            .section
            .checked_sub(1)
            .and_then(|at| models.get_mut(at))
        else {
            return Err(format!(
                "it lists models for section {}, but there are {section_count} sections",
                listed.section
            ));
        };
        for model in listed.models.iter().map(|model| model.trim()) {
            if !model.is_empty() && !slot.iter().any(|known| known == model) {
                slot.push(model.to_string());
            }
        }
    }
    if let Some(at) = models.iter().position(Vec::is_empty) {
        return Err(format!("section {} lists no mental model", at + 1));
    }
    Ok(models)
}

fn checked_questions(
    models: &[Vec<String>],
    raw: &str,
) -> std::result::Result<Vec<ExamQuestion>, String> {
    let dto: QuestionsDto =
        parse_json(raw).map_err(|error| format!("it was not the JSON asked for ({error:#})"))?;
    let section_count = models.len();
    let mut covered: Vec<Vec<bool>> = models.iter().map(|m| vec![false; m.len()]).collect();
    let mut questions = Vec::new();
    for generated in dto.questions {
        if generated.prompt.trim().is_empty()
            || generated.points.is_empty()
            || generated.points.iter().any(|point| point.trim().is_empty())
        {
            return Err("a question has an empty prompt or no key points".to_string());
        }
        let Some(at) = generated
            .section
            .checked_sub(1)
            .filter(|at| *at < section_count)
        else {
            return Err(format!(
                "a question names section {}, but there are {section_count} sections",
                generated.section
            ));
        };
        let model = generated.model.trim().to_string();
        let Some(which) = models[at].iter().position(|listed| *listed == model) else {
            return Err(format!(
                "a question in section {} names the model `{model}`, which that section does \
                 not list",
                at + 1
            ));
        };
        covered[at][which] = true;
        questions.push(ExamQuestion {
            prompt: generated.prompt,
            points: generated.points,
            section: at,
            shape: generated.shape,
            model,
        });
    }
    for (at, section) in covered.iter().enumerate() {
        if let Some(which) = section.iter().position(|done| !done) {
            return Err(format!(
                "section {} lists the model `{}` but asks no question about it",
                at + 1,
                models[at][which]
            ));
        }
    }
    Ok(questions)
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
    Ok(ExamResult::scored(
        questions,
        parsed.grades,
        cfg.pass_threshold,
    ))
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
    if !has_card_fronts(&cards) {
        bail!("the model replied without any cards — try remediating again");
    }
    Ok(cards)
}

pub fn spawn_questions(
    sources: ExamLayers,
    base: Option<PathBuf>,
    sections: Vec<ExamSection>,
    cfg: ExamConfig,
    ask_cfg: AskConfig,
) -> Receiver<Result<Vec<ExamQuestion>, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let r = generate_questions_from(&sources, base.as_deref(), &sections, &cfg, &ask_cfg)
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
    Graded,
    RemediationCards(String),
}

enum Pending {
    Questions(Receiver<Result<Vec<ExamQuestion>, String>>),
    Grade(Receiver<Result<ExamResult, String>>),
    Remediation(Receiver<Result<String, String>>),
}

pub struct Sitting {
    subject: String,
    deck_token: String,
    deck_path: Option<std::path::PathBuf>,
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
    sections: Vec<ExamSection>,
    deck_sections: Vec<ExamSection>,
    unlocked: bool,
}

fn sitting_record(
    sections: &[ExamSection],
    full: bool,
    result: &ExamResult,
) -> crate::store::SittingRecord {
    let count = result.count();
    crate::store::SittingRecord {
        sections: sections
            .iter()
            .zip(&result.sections)
            .filter(|(_, score)| score.total > 0)
            .map(|(section, score)| crate::store::SectionRecord {
                card_ids: section.card_ids.clone(),
                pass: score.passed,
            })
            .collect(),
        full,
        pass: count.pass,
        partial: count.partial,
        total: count.total,
    }
}

impl Sitting {
    pub fn deck_token(&self) -> &str {
        &self.deck_token
    }

    pub fn start(deck: &Deck, strictness: Strictness, cfg: ExamConfig, ask_cfg: AskConfig) -> Self {
        Self::start_with(deck, exam_sections(deck), strictness, cfg, ask_cfg)
    }

    pub fn start_with(
        deck: &Deck,
        sections: Vec<ExamSection>,
        strictness: Strictness,
        cfg: ExamConfig,
        ask_cfg: AskConfig,
    ) -> Self {
        let boundary_error = source_boundary_error(deck);
        let source_ready = boundary_error.is_none();
        let deck_sections = exam_sections(deck);
        let pending = source_ready.then(|| {
            Pending::Questions(spawn_questions(
                examination_sources(deck),
                Some(workspace::content_root(&deck.path)),
                sections.clone(),
                cfg.clone(),
                ask_cfg.clone(),
            ))
        });
        Self {
            subject: deck.subject.clone(),
            deck_token: deck.deck_token.clone().unwrap_or_default(),
            deck_path: Some(deck.path.clone()),
            deck_fingerprints: deck.cards.iter().map(|c| c.block_fingerprint).collect(),
            strictness,
            cfg,
            ask_cfg,
            phase: if source_ready {
                Phase::Generating
            } else {
                Phase::Results
            },
            questions: Vec::new(),
            answers: Vec::new(),
            current: 0,
            result: None,
            pending,
            error: boundary_error,
            pending_since: source_ready.then(crate::time::now_ms),
            remediated_count: None,
            sections,
            deck_sections,
            unlocked: false,
        }
    }

    pub fn sections(&self) -> &[ExamSection] {
        &self.sections
    }

    pub fn full(&self) -> bool {
        self.sections.len() == self.deck_sections.len()
    }

    pub fn unlocked(&self) -> bool {
        self.unlocked
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
        self.elapsed_secs_at(crate::time::now_ms())
    }
    fn elapsed_secs_at(&self, now_ms: u64) -> Option<u64> {
        self.pending_since
            .map(|since| now_ms.saturating_sub(since) / 1000)
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

    fn accept_questions(&mut self, questions: Vec<ExamQuestion>) {
        self.answers = vec![String::new(); questions.len()];
        self.questions = questions;
        self.current = 0;
        self.phase = Phase::Answering;
    }

    pub fn submit(&mut self) {
        if self.phase != Phase::Answering {
            return;
        }
        self.error = None;
        let rx = spawn_grade(
            self.questions.clone(),
            self.answers.clone(),
            self.strictness,
            self.cfg.clone(),
            self.ask_cfg.clone(),
        );
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
        self.phase == Phase::Results && !self.gaps().is_empty()
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
                self.accept_questions(qs);
                None
            }
            Reply::Questions(Err(e)) => {
                self.error = Some(e);
                None
            }
            Reply::Grade(Ok(result)) => {
                let effect = Some(Effect::Graded);
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

    pub fn poll(
        &mut self,
        store: &mut Store,
        now_ms: u64,
        retire_after_days: Option<u32>,
    ) -> ExamPollOutcome {
        let was_pending = self.pending.is_some();
        let effect = self.advance(now_ms);
        let advanced = was_pending && self.pending.is_none();
        let mut store_mutated = false;
        match effect {
            Some(Effect::Graded) => {
                if let Some(result) = &self.result {
                    let record = sitting_record(&self.sections, self.full(), result);
                    self.unlocked = store.record_sitting(
                        &self.deck_token,
                        &self.deck_sections,
                        &record,
                        now_ms,
                    );
                }
                store_mutated = true;
            }
            Some(Effect::RemediationCards(cards)) => {
                store_mutated = true;
                match crate::store::store_remediation_cards(
                    store,
                    self.deck_path.as_deref(),
                    &self.deck_token,
                    &self.deck_fingerprints,
                    &cards,
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
        ExamPollOutcome {
            advanced,
            store_mutated,
        }
    }
}

/// What one poll did: whether the pending grading settled, and whether the
/// store was mutated. The caller owns persistence, so a failed save lands in
/// its `save_error` accounting instead of vanishing here.
pub struct ExamPollOutcome {
    pub advanced: bool,
    pub store_mutated: bool,
}

enum Reply {
    Questions(Result<Vec<ExamQuestion>, String>),
    Grade(Result<ExamResult, String>),
    Remediation(Result<String, String>),
}

fn thread_gone() -> String {
    "the exam helper exited unexpectedly".to_string()
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

#[derive(Default)]
struct LayerSection {
    urls: Vec<String>,
    files: Vec<(String, String)>,
}

pub fn unreadable_exam_entries(deck: &Deck) -> Vec<(String, String)> {
    let root = workspace::content_root(&deck.path);
    deck.exam
        .iter()
        .filter(|src| !deck::is_url(src))
        .filter_map(|src| {
            let path = crate::source::source_path(src, Some(&root))?;
            read_exam_file(&path)
                .err()
                .map(|error| (src.trim().to_string(), format!("{error:#}")))
        })
        .collect()
}

fn read_exam_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("cannot read `exam:` {}", path.display()))
}

fn split_layer(sources: &[String], base: Option<&Path>) -> Result<LayerSection> {
    let mut urls = Vec::new();
    let mut files = Vec::new();
    for src in sources {
        if deck::is_url(src) {
            urls.push(src.clone());
            continue;
        }
        let Some(path) = crate::source::source_path(src, base) else {
            continue;
        };
        let text = read_exam_file(&path)?;
        let declared = src.trim();
        let label = if Path::new(declared).has_root() {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| declared.to_string())
        } else {
            declared.to_string()
        };
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
    Ok(LayerSection { urls, files })
}

fn source_section(sources: &ExamLayers, base: Option<&Path>) -> Result<String> {
    let own = split_layer(&sources.own, base)?;
    let workspace = split_layer(&sources.workspace, base)?;
    if own.urls.is_empty()
        && own.files.is_empty()
        && workspace.urls.is_empty()
        && workspace.files.is_empty()
    {
        bail!("none of the declared `exam:` paths could be read to examine against");
    }

    let mut out = String::new();
    if !own.urls.is_empty() {
        out.push_str(
            "Read these source pages with the WebFetch tool (fetch each once); \
             they are the deck's own sources, the primary grounding for this exam:\n",
        );
        for url in &own.urls {
            out.push_str("  - ");
            out.push_str(url);
            out.push('\n');
        }
    }
    if !workspace.urls.is_empty() {
        out.push_str(
            "Also read these workspace source pages (supporting context, \
             not the primary grounding):\n",
        );
        for url in &workspace.urls {
            out.push_str("  - ");
            out.push_str(url);
            out.push('\n');
        }
    }
    for (label, text) in &own.files {
        out.push_str(&format!(
            "\nSource file `{label}` (a deck source, primary grounding for this exam):\n<<<SOURCE\n{text}\nSOURCE\n"
        ));
    }
    for (label, text) in &workspace.files {
        out.push_str(&format!(
            "\nWorkspace source file `{label}` (supporting context, not the primary grounding):\n<<<SOURCE\n{text}\nSOURCE\n"
        ));
    }
    Ok(out)
}

fn truncate(text: &str) -> (String, bool) {
    if text.len() <= MAX_SOURCE_BYTES {
        return (text.to_string(), false);
    }
    let end = (0..=MAX_SOURCE_BYTES)
        .rev()
        .find(|&end| text.is_char_boundary(end))
        .unwrap_or_default();
    (
        format!("{}\n[... source truncated ...]", &text[..end]),
        true,
    )
}

fn section_scope(sections: &[ExamSection]) -> String {
    let mut scope = String::new();
    for (i, section) in sections.iter().enumerate() {
        scope.push_str(&format!("Section {}: {}\n", i + 1, section.label));
        for line in section.context.iter().skip(1) {
            scope.push_str(line);
            scope.push('\n');
        }
        scope.push_str("Cards:\n");
        for card in &section.cards {
            scope.push_str(&format!("- {}\n", card.replace('\n', "\n  ")));
        }
        scope.push('\n');
    }
    scope
}

fn models_prompt(
    sources: &ExamLayers,
    base: Option<&Path>,
    sections: &[ExamSection],
    cfg: &ExamConfig,
) -> Result<String> {
    let sources = source_section(sources, base)?;
    let scope = section_scope(sections);
    let mut prompt = format!(
        "You are preparing an oral exam that verifies a student truly UNDERSTANDS a \
         topic.\n\n\
         {sources}\n\
         The student drilled a flashcard deck on this material, split into the sections \
         below. Each section's heading, with the prose under it when there is any, marks \
         out its topic. Its cards show only what the student has drilled so far; they do \
         not mark the edge of the topic.\n\n\
         {scope}\
         For EACH section, in deck order, list its mental models: the distinct ideas a \
         student must understand to master that section's topic, as the source above \
         teaches them. Cover everything the source teaches within that topic, whether or \
         not a card states it, and leave out what belongs to another section's topic. \
         List as many as the section really holds, at least one; name each model in a \
         short phrase.\n\n\
         Output ONLY JSON in exactly this shape, no prose, no code fences:\n\
         {{\"sections\": [{{\"section\": 1, \"models\": [\"...\", \"...\"]}}]}}"
    );
    push_extra(&mut prompt, cfg);
    Ok(prompt)
}

fn push_extra(prompt: &mut String, cfg: &ExamConfig) {
    if let Some(extra) = cfg
        .extra
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        prompt.push_str("\n\nAdditional instructions:\n");
        prompt.push_str(extra);
    }
}

fn questions_prompt(
    sources: &ExamLayers,
    base: Option<&Path>,
    sections: &[ExamSection],
    models: &[Vec<String>],
    cfg: &ExamConfig,
) -> Result<String> {
    let sources = source_section(sources, base)?;
    if sections.is_empty() {
        bail!("the deck has no cards to examine");
    }
    let mut scope = String::from(
        "The student drilled a flashcard deck on this material, split into the \
         sections below. Its cards are SCOPE ONLY: they show what the deck already \
         asks. NEVER ask what a card states, and never rephrase a card; test the \
         mental model behind the section instead.\n\n",
    );
    scope.push_str(&section_scope(sections));
    scope.push_str("The mental models to examine, per section:\n");
    for (i, listed) in models.iter().enumerate() {
        scope.push_str(&format!("Section {}:\n", i + 1));
        for model in listed {
            scope.push_str(&format!("- {model}\n"));
        }
    }
    scope.push('\n');
    let ask = format!(
        "{scope}For EACH section, in deck order, write at least one question for every \
         mental model listed for it; a model may get more when it needs them. There is \
         no upper limit. A model a card states is still examined: ask the student to \
         apply it, never to recite the card. Across a section, start with an instance \
         and then take the three shapes in turn:\n\
         - instance: a concrete case the source supports but no card shows \
         (\"given this program, what does the compiler report?\");\n\
         - boundary: when does the section's idea stop holding, or what changes \
         if a condition flips;\n\
         - fix: a short statement or snippet with exactly one seeded mistake about \
         the section's idea; the student must find and correct it.\n\
         Every question must be answerable from the source above and demand \
         reasoning in the student's own words. Tag every question with its section's \
         number, its shape, and the model it tests, spelled exactly as listed for that \
         section.\n\n"
    );
    let shape = "{\"questions\": [{\"section\": 1, \"shape\": \"instance|boundary|fix\", \
                 \"model\": \"<a model listed for that section>\", \"prompt\": \"...\", \
                 \"points\": [\"...\", \"...\"]}]}";
    let mut prompt = format!(
        "You are a strict examiner writing an oral exam that verifies a student \
         truly UNDERSTANDS a topic — not whether they memorized isolated facts.\n\n\
         {sources}\n\
         {ask}\
         {QUESTION_FORM}\
         For each question, list the key points a correct answer MUST contain — \
         the specific, source-grounded facts or reasoning steps you will grade \
         against. Be concrete; these are the rubric.\n\n\
         Output ONLY JSON in exactly this shape, no prose, no code fences around \
         it (a fence inside a `prompt` string is fine):\n\
         {shape}"
    );
    push_extra(&mut prompt, cfg);
    Ok(prompt)
}

const QUESTION_FORM: &str = "\
How each question reads:
- It stands alone. The student sees only the question, never the source: \
reproduce any example, listing or claim you build on in full. Never point at \
what the student cannot see (\"the book's shadowing example\", \"the \
Reference shows\", \"the idiom's form\"); naming the source is fine only \
beside the reproduced content, never instead of it.
- Code goes in a fenced block (```rust and so on) after the prose, one \
statement per line, so the student can refer to its lines; a quoted claim \
goes in a `>` quote. Inline backticks only for a short name or expression \
inside a sentence.
- Write only the finished question: no thinking aloud and no self-correction \
(\"wait\", \"actually\") in the question or in a claim it quotes.
- Never give the answer away. A fix question names neither the mistake, nor \
its line, nor the rule it breaks; any question avoids wording that states \
what the student must work out.

";

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
         cheap recall card. Prefer a cloze: the sentence that states the fact \
         goes on the `## ` line itself, then one \
         `<!-- blank: span hidden=\"...\" -->` comment line per hidden span, \
         where `hidden` repeats the exact words to hide. A cloze card has no \
         separate question, no answer line and no `---`. If there is no \
         natural word to blank out, use a plain `## ` card with the answer \
         on the line below instead.\n\
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
         (unindented) line(s) below it (a cloze card has none); a `> [!NOTE]` line after them opens an \
         optional note (a caveat, example or why it matters) whose body lines \
         each start with `> `. A blockquote with no badge first line is a \
         quotation and belongs to the answer. No frontmatter, no \
         headings other than the `## ` fronts. One example of each card type:\n\
         ## A String stores a pointer, length and capacity on the stack, and its \
         bytes live on the heap.\n\
         <!-- blank: span hidden=\"pointer\" -->\n\
         <!-- blank: span hidden=\"length\" -->\n\
         <!-- blank: span hidden=\"capacity\" -->\n\
         <!-- blank: span hidden=\"heap\" -->\n\
         ## What does `drop` do for a String, and when?\n\
         It returns the String's heap buffer to the allocator, at the end of the \
         owning scope.\n\
         ## Why does moving a String invalidate the original binding?\n\
         Both bindings would otherwise point at the same heap buffer.\n\
         Dropping both would free it twice (a double free).\n\
         So Rust invalidates the source instead of allowing two owners.\n\
         > [!NOTE]\n\
         > A move, not a shallow copy you can keep using.\n\n\
         Before finishing, re-read your cards as a set and merge any two that test \
         the same idea, so every card is distinct.\n\
         Output ONLY the deck text — no markdown code fences, no preamble, no \
         closing remarks.",
    );
    prompt
}

/// A generated block counts as cards when it carries a front at any card
/// depth; a section heading alone is prose.
fn has_card_fronts(text: &str) -> bool {
    text.lines().any(|l| {
        crate::parser::heading_depth(l).is_some_and(|(d, _)| crate::parser::is_card_depth(d))
    })
}

fn clean_deck_output(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let Some(start) = lines.iter().position(|l| {
        crate::parser::heading_depth(l).is_some_and(|(d, _)| crate::parser::is_card_depth(d))
    }) else {
        return raw.trim().to_string();
    };
    let end = lines[start + 1..]
        .iter()
        .rposition(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("```")
        })
        .map(|offset| start + offset + 2)
        .unwrap_or(start + 1);
    lines[start..end].join("\n")
}

#[derive(Debug, Deserialize)]
struct ModelsDto {
    sections: Vec<SectionModelsDto>,
}

#[derive(Debug, Deserialize)]
struct QuestionsDto {
    questions: Vec<GeneratedQuestionDto>,
}

#[derive(Debug, Deserialize)]
struct SectionModelsDto {
    section: usize,
    models: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GeneratedQuestionDto {
    section: usize,
    shape: Shape,
    model: String,
    prompt: String,
    points: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GradesDto {
    grades: Vec<AnswerGrade>,
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn exam_open_excludes_a_new_card_when_stamping_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: deck-d\nexam: source.md\n---\n\
             ## old\nanswer\n<!-- id: card-old -->\n\n\
             ## new\nanswer\n\n```\nunclosed\n",
        )
        .unwrap();

        let deck = open_for_exam(&path).unwrap();

        assert_eq!(
            vec!["old"],
            deck.cards
                .iter()
                .map(|card| card.front.as_str())
                .collect::<Vec<_>>(),
            "the warning promises that unstamped cards are excluded from the exam"
        );
    }

    #[test]
    fn a_workspace_deck_resolves_its_exam_files_from_the_workspace_root_as_the_exam_does() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix.toml"), "title = \"W\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("sources")).unwrap();
        std::fs::write(dir.path().join("sources/notes.md"), "n").unwrap();
        std::fs::create_dir_all(dir.path().join("decks")).unwrap();
        let path = dir.path().join("decks/d.md");
        std::fs::write(&path, "---\nexam: sources/notes.md\n---\n## q\na\n").unwrap();
        crate::stamp::stamp_deck(&path).unwrap();
        let deck = Deck::load(&path).unwrap();
        assert!(
            workspace::root_for_deck(&deck.path).is_some(),
            "setup: the deck is a workspace member"
        );
        assert_eq!(
            Vec::<(String, String)>::new(),
            unreadable_exam_entries(&deck),
            "a file the exam reads from the workspace root is not reported"
        );
    }

    #[test]
    fn unreadable_exam_entries_names_a_directory_and_a_missing_file_but_not_a_file_or_url() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "n").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nexam: [notes.md, src, ghost.md, https://example.test/page]\n---\n## q\na\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();

        let unreadable = unreadable_exam_entries(&deck);

        let names: Vec<&str> = unreadable.iter().map(|(entry, _)| entry.as_str()).collect();
        assert_eq!(
            vec!["src", "ghost.md"],
            names,
            "unreadable entries: {unreadable:#?}"
        );
        assert!(
            unreadable
                .iter()
                .all(|(_, why)| why.starts_with("cannot read `exam:`")),
            "each reason is the exam's own start error: {unreadable:#?}"
        );
    }

    const ONE_SECTION_REPLY: &str = "{\"sections\":[{\"section\":1,\"models\":[\"m\"]}],\
        \"questions\":[{\"section\":1,\"shape\":\"instance\",\"model\":\"m\",\
        \"prompt\":\"Why move?\",\"points\":[\"avoids double free\"]}]}";

    fn deck_with_exam_and_card(srcs: &[&str]) -> Deck {
        Deck {
            cards: parser::parse_str("deck-d", "## Why move?\nownership\n").unwrap(),
            ..deck_with_exam(srcs)
        }
    }

    fn models_for(sections: usize) -> Vec<Vec<String>> {
        vec![vec!["m".to_string()]; sections]
    }

    fn one_section() -> Vec<ExamSection> {
        vec![ExamSection {
            label: "Moves".to_string(),
            context: vec!["Moves".to_string()],
            cards: vec!["Q: a\nA: b".to_string()],
            ..Default::default()
        }]
    }

    fn deck_with_exam(srcs: &[&str]) -> Deck {
        Deck {
            // A root that cannot exist, so no stray manifest supplies a
            // workspace source to these fixtures.
            path: std::path::PathBuf::from("/alix-nonexistent-test-root/d.md"),
            subject: "d.md".to_string(),
            deck_token: None,
            cards: Vec::new(),
            ignored: Vec::new(),
            definitions: Vec::new(),
            links: Vec::new(),
            requires: Vec::new(),
            exam: srcs.iter().map(|s| s.to_string()).collect(),
            cite: Vec::new(),
            settings: Default::default(),
            title: None,
            description: None,
            trace: None,
            workspace_has_exam: Default::default(),
            load_warnings: Vec::new(),
        }
    }

    fn url_layers(own: &[&str]) -> ExamLayers {
        ExamLayers {
            own: own.iter().map(|s| s.to_string()).collect(),
            workspace: Vec::new(),
        }
    }

    /// Generated cards are recognized at every card depth: a check that
    /// only knows `## ` would reject a valid generated sub-card set as
    /// empty, or keep prose above the first front.
    #[test]
    fn generated_card_text_is_recognized_at_every_depth() {
        assert!(has_card_fronts("### only a sub-card\nanswer\n"));
        assert!(has_card_fronts("#### deep\nanswer\n"));
        assert!(!has_card_fronts("# just a section\nprose\n"));
    }

    #[test]
    fn a_workspace_source_joins_deck_sources_as_its_own_labeled_layer() {
        let dir = tempfile::tempdir().unwrap();
        let members = dir.path().join(crate::workspace::DECKS);
        std::fs::create_dir_all(&members).unwrap();
        std::fs::write(
            dir.path().join(crate::workspace::MANIFEST),
            "exam = \"https://example.org/current\"\n",
        )
        .unwrap();
        let path = members.join("d.md");
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: \"deck-deck1\"\nexam: https://example.org/own\n---\n## q\na\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();

        assert_eq!(
            ExamLayers {
                own: vec!["https://example.org/own".to_string()],
                workspace: vec!["https://example.org/current".to_string()],
            },
            examination_sources(&deck)
        );
        assert!(deck.has_exam());
    }

    #[test]
    fn a_workspace_source_duplicated_by_the_deck_is_not_layered_twice() {
        let dir = tempfile::tempdir().unwrap();
        let members = dir.path().join(crate::workspace::DECKS);
        std::fs::create_dir_all(&members).unwrap();
        std::fs::write(
            dir.path().join(crate::workspace::MANIFEST),
            "exam = \"https://example.org/own\"\n",
        )
        .unwrap();
        let path = members.join("d.md");
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: \"deck-deck1\"\nexam: https://example.org/own\n---\n## q\na\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();

        assert_eq!(
            url_layers(&["https://example.org/own"]),
            examination_sources(&deck)
        );
    }

    /// A fact-deck exam grades against the source, never a card's section
    /// context. This is a signature-name tripwire for that boundary, not a
    /// generic ban on deck-derived values: question generation takes the
    /// sections as scope.
    #[test]
    fn exam_prompt_signatures_ban_section_like_names() {
        let source = include_str!("exam.rs");
        let builders: [(&str, &[&str]); 4] = [
            ("fn questions_prompt(", &["sections: &[ExamSection]"]),
            ("fn models_prompt(", &["sections: &[ExamSection]"]),
            ("fn grade_prompt(", &[]),
            ("fn remediation_prompt(", &[]),
        ];
        for (builder, trace_grounding) in builders {
            let at = source
                .find(builder)
                .unwrap_or_else(|| panic!("{builder} moved or was renamed"));
            let rest = &source[at..];
            let end = rest
                .find(" -> ")
                .unwrap_or_else(|| panic!("{builder} signature"));
            let params = &rest[..end];
            let unpinned = trace_grounding
                .iter()
                .fold(params.to_string(), |rest, pinned| rest.replace(pinned, ""));
            for banned in ["context", "section", "Section", "cards", "deck_body"] {
                assert!(
                    !unpinned.contains(banned),
                    "{builder} takes `{banned}`: section/card context must not enter \
                     exam prompt signatures.\n{params}"
                );
            }
            for required in trace_grounding {
                assert!(
                    params.contains(required),
                    "{builder} must expose `{required}` as explicit trace grounding.\n{params}"
                );
            }
        }
    }

    #[test]
    fn questions_prompt_labels_deck_and_workspace_layers() {
        let sources = ExamLayers {
            own: vec!["https://example.org/own".to_string()],
            workspace: vec!["https://example.org/context".to_string()],
        };
        let p = questions_prompt(
            &sources,
            None,
            &one_section(),
            &models_for(1),
            &ExamConfig::default(),
        )
        .unwrap();
        let own_at = p.find("https://example.org/own").unwrap();
        let workspace_at = p.find("https://example.org/context").unwrap();
        assert!(p.contains("primary grounding"));
        assert!(p.contains("supporting context"));
        assert!(own_at < workspace_at, "{p}");
    }

    #[test]
    fn questions_prompt_distinguishes_sources_that_share_a_basename() {
        let dir = tempfile::tempdir().unwrap();
        for (parent, text) in [("a", "alpha source"), ("b", "beta source")] {
            let source_dir = dir.path().join(parent);
            std::fs::create_dir(&source_dir).unwrap();
            std::fs::write(source_dir.join("notes.md"), text).unwrap();
        }
        let sources = ExamLayers {
            own: vec!["a/notes.md".to_string(), "b/notes.md".to_string()],
            workspace: Vec::new(),
        };

        let prompt = questions_prompt(
            &sources,
            Some(dir.path()),
            &one_section(),
            &models_for(1),
            &ExamConfig::default(),
        )
        .unwrap();

        assert!(
            prompt.contains("Source file `a/notes.md`"),
            "the first source is not labeled by what the deck declared: {prompt}"
        );
        assert!(
            prompt.contains("Source file `b/notes.md`"),
            "the second source is not labeled by what the deck declared: {prompt}"
        );
    }

    #[test]
    fn an_absolute_source_is_labeled_by_its_basename_and_a_relative_one_by_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("deep");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("relative.md"), "relative body").unwrap();
        std::fs::write(dir.path().join("absolute.md"), "absolute body").unwrap();
        let absolute = dir.path().join("absolute.md");
        let sources = ExamLayers {
            own: vec![
                "deep/relative.md".to_string(),
                absolute.display().to_string(),
            ],
            workspace: Vec::new(),
        };

        let prompt = questions_prompt(
            &sources,
            Some(dir.path()),
            &one_section(),
            &models_for(1),
            &ExamConfig::default(),
        )
        .unwrap();

        assert!(
            prompt.contains("Source file `deep/relative.md`"),
            "a relative declaration should reach the prompt as declared: {prompt}"
        );
        assert!(
            prompt.contains("Source file `absolute.md`"),
            "an absolute declaration should reach the prompt as its basename: {prompt}"
        );
        assert!(
            !prompt.contains(&absolute.display().to_string()),
            "the operator's directory layout reached the prompt: {prompt}"
        );
    }

    #[test]
    fn same_named_absolute_sources_in_different_layers_have_distinct_headings() {
        let dir = tempfile::tempdir().unwrap();
        let own = dir.path().join("own").join("notes.md");
        let workspace = dir.path().join("workspace").join("notes.md");
        std::fs::create_dir(own.parent().unwrap()).unwrap();
        std::fs::create_dir(workspace.parent().unwrap()).unwrap();
        std::fs::write(&own, "primary body").unwrap();
        std::fs::write(&workspace, "supporting body").unwrap();
        let sources = ExamLayers {
            own: vec![own.display().to_string()],
            workspace: vec![workspace.display().to_string()],
        };

        let prompt = questions_prompt(
            &sources,
            None,
            &one_section(),
            &models_for(1),
            &ExamConfig::default(),
        )
        .unwrap();

        assert_eq!(
            1,
            prompt
                .matches("Source file `notes.md` (a deck source, primary grounding")
                .count(),
            "deck heading: {prompt}"
        );
        assert_eq!(
            1,
            prompt
                .matches("Workspace source file `notes.md` (supporting context")
                .count(),
            "workspace heading: {prompt}"
        );
    }

    #[test]
    fn questions_prompt_carries_source_strictness_and_json_shape() {
        let sources = url_layers(&["https://example.org/ownership"]);
        let p = questions_prompt(
            &sources,
            None,
            &one_section(),
            &models_for(1),
            &ExamConfig::default(),
        )
        .unwrap();
        assert!(p.contains("https://example.org/ownership"));
        assert!(p.contains("WebFetch"));
        assert!(p.contains("strict examiner"));
        assert!(p.contains("\"questions\""));
    }

    #[test]
    fn questions_prompt_asks_per_section_in_the_three_shapes_with_cards_as_scope_only() {
        let sources = url_layers(&["https://example.org/ownership"]);
        let sections = [
            ExamSection {
                label: "# Moves".to_string(),
                context: vec![
                    "# Moves".to_string(),
                    "Values move on assignment.".to_string(),
                ],
                cards: vec!["Q: What does `let b = a;` do to a String?\nA: Moves it".to_string()],
                ..Default::default()
            },
            ExamSection {
                label: "# Borrows".to_string(),
                context: vec!["# Borrows".to_string()],
                cards: vec!["Q: How many &mut at once?\nA: One".to_string()],
                ..Default::default()
            },
        ];
        let p = questions_prompt(
            &sources,
            None,
            &sections,
            &models_for(sections.len()),
            &ExamConfig::default(),
        )
        .unwrap();
        let moves = p.find("# Moves").expect("first section heading");
        let borrows = p.find("# Borrows").expect("second section heading");
        assert!(moves < borrows, "sections keep deck order: {p}");
        assert!(
            p.contains("Values move on assignment."),
            "section prose: {p}"
        );
        assert!(
            p.contains("What does `let b = a;` do to a String?"),
            "card scope: {p}"
        );
        assert!(
            p.contains("NEVER ask what a card states"),
            "scope only: {p}"
        );
        for rule in [
            "The mental models to examine, per section:",
            "at least one question for every \
             mental model listed for it",
            "There is \
             no upper limit.",
            "ask the student to \
             apply it, never to recite the card",
            "spelled exactly as listed for that \
             section",
        ] {
            assert!(p.contains(rule), "models rule {rule:?}: {p}");
        }
        assert!(!p.contains("Write exactly"), "no fixed count: {p}");
        for shape in ["instance:", "boundary:", "fix:"] {
            assert!(p.contains(shape), "shape {shape}: {p}");
        }
        for tag in ["\"section\"", "\"shape\"", "\"model\""] {
            assert!(p.contains(tag), "json carries {tag}: {p}");
        }
    }

    #[test]
    fn exam_sections_group_cards_by_heading_in_deck_order_and_merge_repeated_headings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "# A\n\nAbout a.\n\n## a1?\none\n\n# B\n\n## b1?\ntwo\n\n# A\n\n## a2?\nthree\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        let sections = exam_sections(&deck);
        let headings: Vec<_> = sections
            .iter()
            .map(|s| s.context.first().cloned())
            .collect();
        assert_eq!(
            vec![Some("A".to_string()), Some("B".to_string())],
            headings,
            "same heading text merges; first appearance orders"
        );
        assert_eq!(
            2,
            sections[0].cards.len(),
            "a1 and a2: {:?}",
            sections[0].cards
        );
        assert!(sections[0].cards[0].contains("a1?") && sections[0].cards[0].contains("one"));
    }

    #[test]
    fn section_status_follows_member_records_and_any_fail_wins() {
        use SectionStatus::*;
        type Row = (
            &'static str,
            [Option<bool>; 3],
            [SectionStatus; 2],
            ExamFilter,
        );
        let rows: [Row; 7] = [
            (
                "never examined",
                [None, None, None],
                [Unexamined, Unexamined],
                ExamFilter::All,
            ),
            (
                "all passed",
                [Some(true), Some(true), Some(true)],
                [Passed, Passed],
                ExamFilter::None,
            ),
            (
                "a card without a record among passes",
                [Some(true), None, Some(true)],
                [NewCards, Passed],
                ExamFilter::Partial,
            ),
            (
                "a fail beside a missing record",
                [Some(false), None, Some(true)],
                [NeedsWork, Passed],
                ExamFilter::Partial,
            ),
            (
                "any fail wins over a pass",
                [Some(true), Some(false), Some(true)],
                [NeedsWork, Passed],
                ExamFilter::Partial,
            ),
            (
                "one section unexamined",
                [Some(true), Some(true), None],
                [Passed, Unexamined],
                ExamFilter::Partial,
            ),
            (
                "every section needs work",
                [Some(false), Some(false), Some(false)],
                [NeedsWork, NeedsWork],
                ExamFilter::All,
            ),
        ];
        for (label, records, expected, filter) in rows {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("s.md");
            std::fs::write(
                &path,
                "---\nformat-version: 1\nid: deck-s\nexam: source.txt\n---\n\
                 # A\n\n## a1\nx\n<!-- id: card-a1 -->\n\n## a2\nx\n<!-- id: card-a2 -->\n\n\
                 # B\n\n## b1\nx\n<!-- id: card-b1 -->\n",
            )
            .unwrap();
            let deck = Deck::load(&path).unwrap();
            let mut store = Store::open(dir.path().join("p.json")).unwrap();
            for (id, pass) in ["card-a1", "card-a2", "card-b1"].iter().zip(records) {
                if let Some(pass) = pass {
                    store.get_or_insert(id).exam_result =
                        Some(crate::store::ExamRecord { at_ms: 1, pass });
                }
            }
            let status = deck.exam_status(&store);
            assert_eq!(expected.to_vec(), status.sections, "{label}: statuses");
            assert_eq!(filter, status.filter(), "{label}: filter");
            let tally = status.tally();
            assert_eq!(
                2,
                tally.passed + tally.needs_work + tally.new_cards + tally.unexamined,
                "{label}: the tally counts every section once"
            );
            assert_eq!(
                expected.iter().filter(|s| **s == Passed).count(),
                tally.passed,
                "{label}: passed tally"
            );
            assert_eq!(
                expected == [Passed, Passed],
                status.all_passed(),
                "{label}: all passed"
            );
        }
    }

    #[test]
    fn sections_follow_headings_merge_repeats_and_label_the_unsectioned_with_the_deck_title() {
        let rows = [
            (
                "no heading",
                "## q1?\na\n\n## q2?\nb\n",
                vec![("Ownership", 2)],
            ),
            (
                "two headings",
                "# A\n\n## a1?\none\n\n# B\n\n## b1?\ntwo\n",
                vec![("A", 1), ("B", 1)],
            ),
            (
                "repeated heading",
                "# A\n\n## a1?\none\n\n# B\n\n## b1?\ntwo\n\n# A\n\n## a2?\nthree\n",
                vec![("A", 2), ("B", 1)],
            ),
            (
                "a heading with prose and no cards",
                "## q?\na\n\n# Empty\n\nJust prose.\n\n# B\n\n## b?\nc\n",
                vec![("Ownership", 1), ("B", 1)],
            ),
        ];
        for (label, body, expected) in rows {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("d.md");
            std::fs::write(&path, format!("---\ntitle: Ownership\n---\n\n{body}")).unwrap();
            crate::stamp::stamp_deck(&path).unwrap();
            let deck = Deck::load(&path).unwrap();
            let sections = exam_sections(&deck);
            let got: Vec<(&str, usize)> = sections
                .iter()
                .map(|s| (s.label.as_str(), s.card_ids.len()))
                .collect();
            assert_eq!(expected, got, "{label}: (label, member ids) per section");
            let mut every_id: Vec<String> =
                deck.cards.iter().filter_map(|card| card.id()).collect();
            let mut listed: Vec<String> =
                sections.iter().flat_map(|s| s.card_ids.clone()).collect();
            every_id.sort();
            listed.sort();
            assert_eq!(
                every_id, listed,
                "{label}: every card id belongs to exactly one section"
            );
        }
    }

    #[test]
    fn questions_prompt_asks_for_standalone_fenced_questions_that_hide_the_answer() {
        let sources = url_layers(&["https://example.org/ownership"]);
        for sections in [vec![ExamSection {
            label: "# Moves".to_string(),
            context: vec!["# Moves".to_string()],
            cards: vec!["Q: a\nA: b".to_string()],
            ..Default::default()
        }]] {
            let p = questions_prompt(
                &sources,
                None,
                &sections,
                &models_for(sections.len()),
                &ExamConfig::default(),
            )
            .unwrap();
            for rule in [
                "It stands alone.",
                "Code goes in a fenced block",
                "one statement per line",
                "no thinking aloud and no self-correction",
                "Never give the answer away.",
                "a fence inside a `prompt` string is fine",
            ] {
                assert!(
                    p.contains(rule),
                    "{} sections: missing {rule:?}",
                    sections.len()
                );
            }
        }
    }

    #[test]
    fn models_prompt_bounds_the_topic_by_the_heading_and_shows_cards_only_as_what_is_drilled() {
        let sources = url_layers(&["https://x"]);
        let p = models_prompt(&sources, None, &one_section(), &ExamConfig::default()).unwrap();
        assert!(
            p.contains("heading, with the prose under it when there is any, marks out its topic"),
            "the heading bounds the topic: {p}"
        );
        assert!(
            p.contains(
                "cards show only what the student has drilled so far; they do not mark the edge"
            ),
            "the cards are not the topic's boundary: {p}"
        );
        assert!(
            !p.contains("heading and cards mark out"),
            "no card may narrow the topic: {p}"
        );
    }

    #[test]
    fn questions_prompt_honors_extra() {
        let sources = url_layers(&["https://x"]);
        let cfg = ExamConfig {
            extra: Some("Focus on lifetimes.".to_string()),
            ..ExamConfig::default()
        };
        let p = questions_prompt(&sources, None, &one_section(), &models_for(1), &cfg).unwrap();
        assert!(p.contains("Additional instructions:"));
        assert!(p.contains("Focus on lifetimes."));
    }

    #[test]
    fn exam_extra_reaches_the_model_listing_call_that_decides_coverage() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(
            dir.path(),
            &[
                "{\"sections\":[{\"section\":1,\"models\":[\"ownership\"]}]}",
                "{\"questions\":[{\"section\":1,\"shape\":\"instance\",\
                 \"model\":\"ownership\",\"prompt\":\"Q\",\"points\":[\"p\"]}]}",
            ],
        );
        let cfg = ExamConfig {
            extra: Some("Focus on lifetimes.".to_string()),
            ..ExamConfig::default()
        };
        let deck = deck_with_exam_and_card(&["https://x"]);

        generate_questions(&deck, &cfg, &ask_config(&cli)).unwrap();

        let first = std::fs::read_to_string(dir.path().join("prompt-1")).unwrap();
        assert!(
            first.contains("Focus on lifetimes."),
            "the first call fixes the allowed model list, so exam guidance must reach it: {first}"
        );
    }

    #[test]
    fn grade_prompt_includes_rubric_and_answers() {
        let qs = vec![ExamQuestion {
            prompt: "Why move?".to_string(),
            points: vec!["avoids double free".to_string()],
            ..Default::default()
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
            ..Default::default()
        }];
        let p = grade_prompt(&qs, &["   ".to_string()], Strictness::Balanced);
        assert!(p.contains("(no answer given)"));
    }

    #[test]
    fn grade_prompt_criteria_vary_by_strictness() {
        let qs = vec![ExamQuestion {
            prompt: "Q".to_string(),
            points: vec!["p".to_string()],
            ..Default::default()
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
    fn remediation_prompt_example_writes_a_badged_note() {
        let prompt = remediation_prompt(&["the aliasing rule".to_string()]);

        assert!(
            prompt.contains("> [!NOTE]\n> A move, not a shallow copy you can keep using."),
            "the example must model the note grammar the prose requires: {prompt}"
        );
    }

    #[test]
    fn remediation_prompt_picks_card_type_per_gap() {
        let p = remediation_prompt(&["x".to_string()]);
        assert!(p.contains("missed FACT or TERM"));
        assert!(p.contains("missed CONCEPT"));
        assert!(p.contains("<!-- blank: span hidden="));
        assert!(!p.contains("\\blank{"));
        assert!(p.contains("understanding card"));
        assert!(!p.contains("indented answer"));
        assert!(p.contains("## "));
    }

    #[test]
    fn remediation_prompt_teaches_an_answerless_cloze_card() {
        crate::testutil::assert_teaches_answerless_cloze(
            "remediation prompt",
            &remediation_prompt(&["x".to_string()]),
        );
    }

    #[test]
    fn remediation_prompt_recall_example_parses_as_four_blank_cards() {
        let prompt = remediation_prompt(&["x".to_string()]);
        let start = prompt
            .find("## A String stores a pointer, length and capacity")
            .expect("the recall example starts");
        let tail = &prompt[start..];
        let end = tail
            .find("\n## What does `drop` do for a String, and when?")
            .expect("the next example bounds the recall card");
        let cards = crate::parser::parse_str("model-output.md", &tail[..end]).unwrap();

        assert_eq!(
            4,
            cards.len(),
            "the prompt's four hidden facts must become four review cards: {cards:?}"
        );
        assert!(
            cards.iter().all(|card| card.region.is_some()),
            "every generated recall card must be a span-derived card: {cards:?}"
        );
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
        let raw = "Here you go:\n```json\n{\"sections\": [{\"section\": 1, \"models\": [\"m\"]}], \
                   \"questions\": [{\"section\": 1, \"shape\": \"fix\", \"model\": \"m\", \
                   \"prompt\": \"Q1\", \"points\": [\"a\", \"b\"]}]}\n```";
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
    fn section_pass_and_sitting_count_follow_the_verdicts() {
        use Verdict::*;
        type Row = (
            &'static str,
            &'static [(usize, Verdict)],
            f64,
            [bool; 2],
            (u32, u32, u32),
            bool,
        );
        let rows: [Row; 5] = [
            (
                "all PASS",
                &[(0, Pass), (1, Pass)],
                1.0,
                [true, true],
                (2, 0, 2),
                true,
            ),
            (
                "a PARTIAL fails its section at 1.0",
                &[(0, Pass), (0, Partial), (1, Pass)],
                1.0,
                [false, true],
                (2, 1, 3),
                false,
            ),
            (
                "a PARTIAL passes its section at 0.5",
                &[(0, Pass), (0, Partial), (1, Pass)],
                0.5,
                [true, true],
                (2, 1, 3),
                true,
            ),
            (
                "a fail in one section leaves the other passed",
                &[(0, Fail), (1, Pass)],
                1.0,
                [false, true],
                (1, 0, 2),
                false,
            ),
            (
                "an unexamined section is not scored",
                &[(1, Pass)],
                1.0,
                [false, true],
                (1, 0, 1),
                true,
            ),
        ];
        for (label, graded, threshold, passes, count, _) in rows {
            let questions: Vec<ExamQuestion> = graded
                .iter()
                .map(|(section, _)| ExamQuestion {
                    section: *section,
                    ..Default::default()
                })
                .collect();
            let grades: Vec<AnswerGrade> = graded
                .iter()
                .map(|(_, verdict)| AnswerGrade {
                    verdict: *verdict,
                    feedback: String::new(),
                    missed: Vec::new(),
                })
                .collect();
            let result = ExamResult::scored(&questions, grades, threshold);
            let sections: Vec<bool> = result.sections.iter().map(|s| s.passed).collect();
            assert_eq!(passes.to_vec(), sections, "{label}: section passes");
            let total = result.count();
            assert_eq!(
                count,
                (total.pass, total.partial, total.total),
                "{label}: count"
            );
            assert_eq!(
                count.0 == count.2,
                total.passed,
                "{label}: the count is full marks only when every answer is PASS"
            );
        }
    }

    #[test]
    fn gaps_collects_missed_from_non_passes_deduped() {
        let result = ExamResult {
            sections: Vec::new(),
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

    #[test]
    fn deck_cleanup_handles_empty_and_trailing_boundaries() {
        assert_eq!("## Q", clean_deck_output("preamble\n## Q\n\n```\n"));
        assert_eq!(
            "## Q\n```not trailing\nA",
            clean_deck_output("preamble\n## Q\n```not trailing\nA\n```\n\n")
        );
    }

    #[test]
    fn source_truncation_is_exact_and_utf8_safe() {
        let exact = "a".repeat(MAX_SOURCE_BYTES);
        assert_eq!((exact.clone(), false), truncate(&exact));

        let mut oversized = "a".repeat(MAX_SOURCE_BYTES - 1);
        oversized.push('é');
        let (truncated, did_truncate) = truncate(&oversized);
        assert!(did_truncate);
        assert_eq!(
            format!(
                "{}\n[... source truncated ...]",
                "a".repeat(MAX_SOURCE_BYTES - 1)
            ),
            truncated
        );
    }

    #[test]
    fn questions_prompt_tells_the_examiner_when_a_source_was_truncated() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("large.md"),
            "a".repeat(MAX_SOURCE_BYTES + 1),
        )
        .unwrap();
        let sources = ExamLayers {
            own: vec!["large.md".to_string()],
            workspace: Vec::new(),
        };

        let prompt = questions_prompt(
            &sources,
            Some(dir.path()),
            &one_section(),
            &models_for(1),
            &ExamConfig::default(),
        )
        .unwrap();

        assert!(
            prompt.contains("[... source truncated ...]"),
            "the pasted source carries no truncation marker, so the examiner reads a partial source as whole"
        );
        assert!(
            prompt.contains("Every question must be answerable from the source"),
            "the answerability instruction is gone, so the marker guards nothing"
        );
    }

    use crate::{
        parser,
        session::is_retired_id,
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
        let deck = deck_with_exam(&[]);
        let err = generate_questions(
            &deck,
            &ExamConfig::default(),
            &ask_config(std::path::Path::new("unused")),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("no `exam:`"));
    }

    #[cfg(unix)]
    #[test]
    fn the_exam_reads_only_exam_entries_never_cite_roots_or_cited_text() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        for folder in ["decks", "code", "lib"] {
            std::fs::create_dir_all(ws.join(folder)).unwrap();
        }
        std::fs::write(
            ws.join(crate::workspace::MANIFEST),
            "exam = \"manifest-exam.txt\"\n\n[cite]\nshared = \"lib\"\n",
        )
        .unwrap();
        std::fs::write(ws.join("manifest-exam.txt"), "MANIFEST_EXAM_MARKER\n").unwrap();
        std::fs::write(ws.join("own-exam.txt"), "DECK_EXAM_MARKER\n").unwrap();
        std::fs::write(ws.join("code/cited.rs"), "CITED_TEXT_MARKER\n").unwrap();
        std::fs::write(ws.join("code/uncited.rs"), "CITE_ROOT_MARKER\n").unwrap();
        std::fs::write(ws.join("lib/shared.rs"), "MANIFEST_CITED_MARKER\n").unwrap();
        let deck_path = ws.join("decks/d.md");
        std::fs::write(
            &deck_path,
            "---\nformat-version: 1\nid: deck-examcitea1b2c3d4e5f6g7h8j9\nexam: own-exam.txt\n\
             cite:\n  - code: code\n---\n\
             ## q1\na\n<!-- at: code:cited.rs:1 -->\n\n\
             ## q2\nb\n<!-- at: shared:shared.rs:1 -->\n",
        )
        .unwrap();
        crate::assets::freeze_member(&deck_path).unwrap();
        crate::stamp::stamp_deck(&deck_path).unwrap();
        let deck = Deck::load(&deck_path).unwrap();
        assert!(
            deck.cards
                .iter()
                .all(|card| card.citations.iter().all(|c| c.asset.is_some())),
            "setup: every citation is frozen"
        );

        let prompt = dir.path().join("prompt.txt");
        let args = dir.path().join("args.txt");
        std::fs::write(dir.path().join("reply.json"), ONE_SECTION_REPLY).unwrap();
        let cli = crate::testutil::fake_cli(
            dir.path(),
            &format!(
                "cat > {}; printf '%s\\n' \"$@\" > {}; cat {}",
                prompt.display(),
                args.display(),
                dir.path().join("reply.json").display()
            ),
        );
        let mut ask_cfg = ask_config(&cli);
        ask_cfg.source_access = true;
        generate_questions(&deck, &ExamConfig::default(), &ask_cfg).unwrap();
        let seen = format!(
            "{}\n{}",
            std::fs::read_to_string(&prompt).unwrap(),
            std::fs::read_to_string(&args).unwrap()
        );

        let code_root = ws.join("code").canonicalize().unwrap();
        let lib_root = ws.join("lib").canonicalize().unwrap();
        let rows: [(&str, &str, bool); 7] = [
            ("the deck's own exam entry", "DECK_EXAM_MARKER", true),
            ("the manifest's exam entry", "MANIFEST_EXAM_MARKER", true),
            ("text cited from a deck root", "CITED_TEXT_MARKER", false),
            (
                "text cited from a manifest root",
                "MANIFEST_CITED_MARKER",
                false,
            ),
            (
                "an uncited file under a cite root",
                "CITE_ROOT_MARKER",
                false,
            ),
            (
                "the deck's cite root path",
                code_root.to_str().unwrap(),
                false,
            ),
            (
                "the manifest's cite root path",
                lib_root.to_str().unwrap(),
                false,
            ),
        ];
        for (label, needle, expected) in rows {
            assert_eq!(
                expected,
                seen.contains(needle),
                "{label}: `{needle}` in the exam's prompt and arguments:\n{seen}"
            );
        }
    }

    #[test]
    fn a_deck_is_exam_eligible_only_when_it_or_its_manifest_declares_exam() {
        let rows = [
            ("cites but declares no exam anywhere", "", "", false),
            ("the deck declares exam", "exam: e.txt\n", "", true),
            ("the manifest declares exam", "", "exam = \"e.txt\"\n", true),
            (
                "both declare exam",
                "exam: e.txt\n",
                "exam = \"e.txt\"\n",
                true,
            ),
        ];
        for (label, deck_exam, manifest_exam, expected) in rows {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("decks")).unwrap();
            std::fs::create_dir_all(dir.path().join("code")).unwrap();
            std::fs::write(dir.path().join("code/x.rs"), "x\n").unwrap();
            std::fs::write(dir.path().join("e.txt"), "e\n").unwrap();
            std::fs::write(
                dir.path().join(crate::workspace::MANIFEST),
                format!("{manifest_exam}\n[cite]\nshared = \"code\"\n"),
            )
            .unwrap();
            let path = dir.path().join("decks/d.md");
            std::fs::write(
                &path,
                format!("---\ntitle: d\n{deck_exam}---\n## q\na\n<!-- at: shared:x.rs:1 -->\n"),
            )
            .unwrap();
            let deck = Deck::load(&path).unwrap();
            assert_eq!(expected, deck.has_exam(), "{label}");
        }
    }

    #[test]
    fn workspace_asset_boundary_failures_surface_through_the_sitting() {
        let dir = tempfile::tempdir().unwrap();
        let members = dir.path().join(crate::workspace::DECKS);
        std::fs::create_dir(&members).unwrap();
        std::fs::write(dir.path().join(crate::workspace::MANIFEST), "").unwrap();
        let path = members.join("d.md");
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: deck-9w2c7x4k1m8q3z5t0v6b2n4d8f\nexam: https://example.org/source\ncite:\n  - src: https://example.org/source\n---\n## q\na\n<!-- at: src fingerprint: xxh64-0000000000000007 asset: missing.txt -->\n<!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();

        let boundary = source_boundary_error(&deck).unwrap();
        assert!(boundary.contains("missing.txt"), "{boundary}");
        let sitting = Sitting::start(
            &deck,
            Strictness::Lenient,
            ExamConfig::default(),
            AskConfig::default(),
        );
        assert_eq!(Strictness::Lenient, sitting.strictness());
        assert_eq!(Some(boundary.as_str()), sitting.error());
        assert_eq!(None, sitting.elapsed_secs());
    }

    #[test]
    fn sitting_navigation_accepts_only_question_indices() {
        let mut sitting = test_sitting(Strictness::Strict);
        sitting.accept_questions(vec![
            ExamQuestion {
                prompt: "first".to_string(),
                points: vec!["one".to_string()],
                ..Default::default()
            },
            ExamQuestion {
                prompt: "second".to_string(),
                points: vec!["two".to_string()],
                ..Default::default()
            },
        ]);

        assert_eq!(Strictness::Strict, sitting.strictness());
        assert_eq!(0, sitting.current_index());
        sitting.next();
        assert_eq!(1, sitting.current_index());
        sitting.next();
        assert_eq!(1, sitting.current_index());
        sitting.goto(0);
        assert_eq!(0, sitting.current_index());
        sitting.goto(2);
        assert_eq!(0, sitting.current_index());
    }

    #[test]
    fn elapsed_seconds_use_a_saturating_millisecond_boundary() {
        let mut sitting = test_sitting(Strictness::Balanced);
        assert_eq!(None, sitting.elapsed_secs_at(3_500));
        sitting.pending_since = Some(1_000);
        assert_eq!(Some(2), sitting.elapsed_secs_at(3_500));
        assert_eq!(Some(0), sitting.elapsed_secs_at(500));
        sitting.pending_since = Some(crate::time::now_ms().saturating_sub(2_500));
        assert!(sitting.elapsed_secs().is_some_and(|elapsed| elapsed >= 2));
    }

    fn test_sitting(strictness: Strictness) -> Sitting {
        Sitting {
            subject: "deck.md".to_string(),
            deck_token: "deck-test".to_string(),
            deck_path: Some(PathBuf::from("deck.md")),
            deck_fingerprints: HashSet::new(),
            strictness,
            cfg: ExamConfig::default(),
            ask_cfg: AskConfig::default(),
            phase: Phase::Answering,
            questions: vec![ExamQuestion {
                prompt: "first".to_string(),
                points: vec!["point".to_string()],
                ..Default::default()
            }],
            answers: vec![String::new()],
            current: 0,
            result: None,
            pending: None,
            error: None,
            pending_since: None,
            remediated_count: None,
            sections: Vec::new(),
            deck_sections: Vec::new(),
            unlocked: false,
        }
    }

    #[test]
    fn exam_run_configuration_overrides_only_its_owned_fields() {
        let ask = AskConfig {
            model: Some("fallback".to_string()),
            timeout_secs: 12,
            cwd: Some(PathBuf::from("/source")),
            source_access: true,
            effort: Some("high".to_string()),
            ..AskConfig::default()
        };
        let exam = ExamConfig {
            model: Some("exam-model".to_string()),
            timeout_secs: 34,
            ..ExamConfig::default()
        };

        let actual = run_config(&exam, &ask);

        assert_eq!(Some("exam-model".to_string()), actual.model);
        assert_eq!(34, actual.timeout_secs);
        assert_eq!(None, actual.cwd);
        assert!(!actual.source_access);
        assert_eq!(Some("high".to_string()), actual.effort);
    }

    #[test]
    fn disconnected_exam_workers_have_a_specific_error() {
        assert_eq!("the exam helper exited unexpectedly", thread_gone());
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
            url_layers(&["https://example.org/doc"]),
            None,
            Vec::new(),
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
        let deck = deck_with_exam(&["https://example.org/doc"]);
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
    fn generation_lists_models_then_questions_and_retries_each_call_once() {
        let _lock = exec_lock();
        let models = "{\"sections\":[{\"section\":1,\"models\":[\"m\",\"n\"]}]}".to_string();
        let no_models = "{\"sections\":[]}".to_string();
        let q = |section: usize, model: &str, prompt: &str, points: &str| {
            format!(
                "{{\"section\":{section},\"shape\":\"fix\",\"model\":\"{model}\",\
                 \"prompt\":\"{prompt}\",\"points\":[{points}]}}"
            )
        };
        let questions = |items: &[String]| format!("{{\"questions\":[{}]}}", items.join(","));
        let good = questions(&[q(1, "m", "Q1", "\"p\""), q(1, "n", "Q2", "\"p\"")]);
        let only_m = questions(&[q(1, "m", "Q", "\"p\"")]);
        let rows: Vec<(&str, Vec<String>, usize, Option<&str>)> = vec![
            (
                "good first time",
                vec![models.clone(), good.clone()],
                2,
                None,
            ),
            (
                "models bad then good",
                vec![no_models.clone(), models.clone(), good.clone()],
                3,
                None,
            ),
            (
                "questions bad then good",
                vec![models.clone(), only_m.clone(), good.clone()],
                3,
                None,
            ),
            (
                "no model listed, twice",
                vec![
                    no_models.clone(),
                    "{\"sections\":[{\"section\":1,\"models\":[\" \"]}]}".to_string(),
                ],
                2,
                Some("the listed models were rejected twice: section 1 lists no mental model"),
            ),
            (
                "models for a section that does not exist, twice",
                vec![
                    "{\"sections\":[{\"section\":2,\"models\":[\"m\"]}]}".to_string(),
                    "{\"sections\":[{\"section\":0,\"models\":[\"m\"]}]}".to_string(),
                ],
                2,
                Some("lists models for section 0"),
            ),
            (
                "a question outside every section, twice",
                vec![
                    models.clone(),
                    questions(&[q(2, "m", "Q", "\"p\"")]),
                    questions(&[q(0, "m", "Q", "\"p\"")]),
                ],
                3,
                Some("the generated questions were rejected twice: a question names section 0"),
            ),
            (
                "an unlisted model, twice",
                vec![
                    models.clone(),
                    questions(&[q(1, "other", "Q", "\"p\"")]),
                    questions(&[q(1, "other", "Q", "\"p\"")]),
                ],
                3,
                Some("names the model `other`"),
            ),
            (
                "an uncovered model, twice",
                vec![models.clone(), only_m.clone(), only_m.clone()],
                3,
                Some("lists the model `n` but asks no question"),
            ),
            (
                "an empty prompt, then no key points",
                vec![
                    models.clone(),
                    questions(&[q(1, "m", " ", "\"p\""), q(1, "n", "Q", "\"p\"")]),
                    questions(&[q(1, "m", "Q", ""), q(1, "n", "Q", "\"p\"")]),
                ],
                3,
                Some("empty prompt or no key points"),
            ),
            (
                "a blank key point, twice",
                vec![
                    models.clone(),
                    questions(&[q(1, "m", "Q", "\" \""), q(1, "n", "Q", "\"p\"")]),
                    questions(&[q(1, "m", "Q", "\"\""), q(1, "n", "Q", "\"p\"")]),
                ],
                3,
                Some("empty prompt or no key points"),
            ),
        ];
        for (label, replies, calls, failure) in rows {
            let dir = tempfile::tempdir().unwrap();
            let replies: Vec<&str> = replies.iter().map(String::as_str).collect();
            let cli = crate::testutil::fake_replies(dir.path(), &replies);
            let deck = deck_with_exam_and_card(&["https://x"]);
            let result = generate_questions(&deck, &ExamConfig::default(), &ask_config(&cli));
            assert_eq!(
                calls,
                crate::testutil::fake_reply_calls(dir.path()),
                "{label}: calls"
            );
            match (failure, result) {
                (None, Ok(questions)) => {
                    let tested: Vec<&str> = questions.iter().map(|q| q.model.as_str()).collect();
                    assert_eq!(
                        vec!["m", "n"],
                        tested,
                        "{label}: every listed model is tested"
                    );
                }
                (Some(want), Err(error)) => {
                    let message = format!("{error:#}");
                    assert!(message.contains(want), "{label}: {message}");
                }
                (failure, result) => panic!("{label}: expected {failure:?}, got {result:?}"),
            }
        }
    }

    #[test]
    fn the_question_call_gets_the_listed_models_and_a_retry_names_the_rejection() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(
            dir.path(),
            &[
                "{\"sections\":[]}",
                "{\"sections\":[{\"section\":1,\"models\":[\"moves transfer ownership\"]}]}",
                "{\"questions\":[{\"section\":1,\"shape\":\"instance\",\
                 \"model\":\"moves transfer ownership\",\"prompt\":\"Q\",\"points\":[\"p\"]}]}",
            ],
        );
        let deck = deck_with_exam_and_card(&["https://x"]);
        generate_questions(&deck, &ExamConfig::default(), &ask_config(&cli)).unwrap();
        let read =
            |n: usize| std::fs::read_to_string(dir.path().join(format!("prompt-{n}"))).unwrap();
        assert!(
            read(1).contains("list its mental models"),
            "call 1 lists models: {}",
            read(1)
        );
        assert!(
            read(2).contains("Your previous answer was rejected: section 1 lists no mental model"),
            "call 2 retries the listing with the reason: {}",
            read(2)
        );
        assert!(
            read(3).contains("- moves transfer ownership"),
            "call 3 writes questions for the listed models: {}",
            read(3)
        );
    }

    #[test]
    fn a_deck_without_cards_is_not_examined() {
        let deck = deck_with_exam(&["https://x"]);
        let error = generate_questions(
            &deck,
            &ExamConfig::default(),
            &ask_config(std::path::Path::new("unused")),
        )
        .unwrap_err();
        assert!(
            format!("{error:#}").contains("no cards to examine"),
            "{error:#}"
        );
    }

    #[test]
    fn grade_answers_rejects_a_count_mismatch() {
        let qs = vec![
            ExamQuestion {
                prompt: "Q1".to_string(),
                points: vec!["p".to_string()],
                ..Default::default()
            },
            ExamQuestion {
                prompt: "Q2".to_string(),
                points: vec!["p".to_string()],
                ..Default::default()
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
                ..Default::default()
            },
            ExamQuestion {
                prompt: "Q2".to_string(),
                points: vec!["p".to_string()],
                ..Default::default()
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
        let cli = fake_reply(dir.path(), ONE_SECTION_REPLY);
        let deck = deck_with_exam_and_card(&["https://x"]);
        let qs = generate_questions(&deck, &ExamConfig::default(), &ask_config(&cli)).unwrap();
        assert_eq!(1, qs.len());
        assert_eq!("Why move?", qs[0].prompt);
        assert_eq!(vec!["avoids double free"], qs[0].points);
        assert_eq!(
            (0, Shape::Instance, "m"),
            (qs[0].section, qs[0].shape, qs[0].model.as_str()),
            "the model's 1-based section becomes an index; shape and model are kept"
        );
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
            ..Default::default()
        }];
        let result = grade_answers(
            &qs,
            &["my answer".to_string()],
            Strictness::Balanced,
            &ExamConfig::default(),
            &ask_config(&cli),
        )
        .unwrap();
        assert!(result.count().passed);
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
            ignored: Vec::new(),
            definitions: Vec::new(),
            links: Vec::new(),
            requires: Vec::new(),
            exam: vec!["notes.md".to_string()],
            cite: Vec::new(),
            settings: Default::default(),
            title: None,
            description: None,
            trace: None,
            workspace_has_exam: Default::default(),
            load_warnings: Vec::new(),
        };
        let section = source_section(&deck.exam_layers(), deck.path.parent()).unwrap();
        assert!(section.contains("the ground truth text"));
        assert!(section.contains("notes.md"));
        assert!(section.contains("primary grounding"));
        std::fs::remove_dir_all(&dir).ok();
    }

    fn drain(s: &mut Sitting, store: &mut Store) {
        for _ in 0..500 {
            if s.poll(store, 0, Some(crate::session::DEFAULT_RETIRE_AFTER_DAYS))
                .advanced
            {
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
             *'\"models\"'*) printf '%s' '{{\"sections\":[{{\"section\":1,\"models\":[\"m\"]}}]}}' ;;\n\
             *'\"questions\"'*) printf '%s' '{{\"sections\":[{{\"section\":1,\"models\":[\"m\"]}}],\"questions\":[{{\"section\":1,\"shape\":\"instance\",\"model\":\"m\",\"prompt\":\"Q1\",\"points\":[\"p1\"]}},{{\"section\":1,\"shape\":\"boundary\",\"model\":\"m\",\"prompt\":\"Q2\",\"points\":[\"p2\"]}}]}}' ;;\n\
             *) printf '## Why does X?\\npoint one\\n' ;;\n\
             esac"
        );
        fake_cli(dir, &body)
    }

    fn sidecar_cards(deck_path: &std::path::Path, deck_id: &str) -> Vec<crate::card::Card> {
        let text =
            std::fs::read_to_string(crate::personal::sidecar_path(deck_path)).unwrap_or_default();
        parser::parse_str(deck_id, &text).unwrap_or_default()
    }

    fn sourced_deck(dir: &std::path::Path) -> Deck {
        let path = dir.join("d.md");
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: \"deck-d1\"\nexam: https://x\n---\n## c\na\n<!-- id: card-qc -->\n",
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
        assert!(!s.result().unwrap().count().passed);
        assert_eq!(vec!["the move rule"], s.gaps());
        assert!(!store.deck_passed("deck-d1"));
        assert_eq!(
            Some(false),
            store
                .get("card-qc")
                .and_then(|c| c.exam_result)
                .map(|r| r.pass),
            "a failed section is recorded on its card"
        );
        assert!(!s.unlocked());

        assert!(s.can_remediate());
        let snapshot = std::fs::read(dir.path().join("d.md")).unwrap();
        s.remediate();
        assert_eq!(&Phase::Remediating, s.phase());
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Remediated, s.phase());
        assert_eq!(snapshot, std::fs::read(dir.path().join("d.md")).unwrap());
        let personal = sidecar_cards(&dir.path().join("d.md"), "deck-d1");
        assert_eq!(1, personal.len());
        assert_eq!("Why does X?", personal[0].front);
        assert_eq!(vec!["point one".to_string()], personal[0].back);
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

        let personal = sidecar_cards(&dir.path().join("d.md"), "deck-d1");
        assert_eq!(1, personal.len());
        assert_eq!(Some(personal.len()), s.remediated_count());
    }

    #[test]
    fn a_settled_sitting_records_through_record_exam_result() {
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
        assert!(s.result().unwrap().count().passed);
        assert!(
            store.deck_passed("deck-d1"),
            "every section passed: passed_at_ms"
        );
        assert!(
            store.deck_mastered("deck-d1"),
            "a full sitting at 100%: mastered_at_ms"
        );
        assert!(s.unlocked(), "this sitting set passed_at_ms");
        assert_eq!(
            Some(true),
            store
                .get("card-qc")
                .and_then(|c| c.exam_result)
                .map(|r| r.pass),
            "the section's verdict is on its card"
        );
        assert_eq!(
            Some((2, 0, 2)),
            store
                .full_exam_result("deck-d1")
                .map(|f| (f.pass, f.partial, f.total)),
            "the full count"
        );
        assert!(!s.can_remediate());
    }

    #[test]
    fn a_failed_exam_writes_remediation_to_the_sidecar_and_leaves_the_deck_unchanged() {
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

        let sidecar = std::fs::read_to_string(crate::personal::sidecar_path(&deck_path))
            .expect("remediation wrote a sidecar beside the deck");
        assert!(
            sidecar.contains("for: deck-d1"),
            "the sidecar names its deck: {sidecar}"
        );
        let cards = crate::parser::parse_str("deck-d1", &sidecar).unwrap();
        assert_eq!(1, cards.len(), "one remediation card: {sidecar}");
        assert!(
            cards[0].id().is_some_and(|id| store.get(&id).is_some()),
            "the store carries the card's schedule, the file carries its content"
        );
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
        assert!(!s.result().unwrap().count().passed);
        s.remediate();
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Remediated, s.phase());
        assert!(!sidecar_cards(&dir.path().join("d.md"), "deck-d1").is_empty());

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
        assert!(s2.result().unwrap().count().passed);
        assert!(store.deck_passed("deck-d1"));

        for card in sidecar_cards(&dir.path().join("d.md"), "deck-d1") {
            let id = card.id().expect("a personal card carries its id");
            assert!(
                !is_retired_id(&id, &store, Some(crate::session::DEFAULT_RETIRE_AFTER_DAYS)),
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
    fn advance_reports_a_failed_grade_for_recording() {
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
        assert_eq!(Some(Effect::Graded), effect, "a fail is recorded too");
        assert_eq!(&Phase::Results, s.phase());
        assert!(!s.result().unwrap().count().passed);
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
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn a_windows_root_relative_source_does_not_send_its_directory_layout() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("private").join("notes.md");
        std::fs::create_dir(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "private body").unwrap();
        let prefix_len = source.components().next().unwrap().as_os_str().len();
        let declared = source.to_string_lossy()[prefix_len..].to_string();
        assert!(Path::new(&declared).has_root(), "{declared}");
        assert!(!Path::new(&declared).is_absolute(), "{declared}");
        let sources = ExamLayers {
            own: vec![declared.clone()],
            workspace: Vec::new(),
        };

        let section = ExamSection {
            label: "Moves".to_string(),
            context: vec!["Moves".to_string()],
            ..Default::default()
        };
        let prompt = questions_prompt(
            &sources,
            Some(dir.path()),
            &[section],
            &[vec!["m".to_string()]],
            &ExamConfig::default(),
        )
        .unwrap();

        assert!(
            prompt.contains("Source file `notes.md`"),
            "a drive-rooted declaration should reach the prompt as its basename: {prompt}"
        );
        assert!(
            !prompt.contains(&declared),
            "the operator's root-relative directory layout reached the prompt: {prompt}"
        );
    }
}
