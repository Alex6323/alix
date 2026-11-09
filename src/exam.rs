use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, TryRecvError, channel},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::{
    ask::{self, parse_json},
    config::{AskConfig, ExamConfig, Strictness},
    deck::{self, Deck},
    exam_ideas::{Idea, IdeaTopic, IdeasDocument},
    sources::DeckSources,
    store::Store,
    workspace,
};
pub use crate::{
    exam_status::{
        ExamFilter, ExamScope, ExamStatus, ExamTopic, SittingStart, TOPIC_NUDGE_CARDS, TopicStatus,
        TopicTally, exam_status, exam_topics, examined_topics, oversized_topics,
        retire_after_days_for, sitting_start,
    },
    sources::SourceText,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExamQuestion {
    pub prompt: String,
    pub points: Vec<String>,
    pub topic: usize,
    pub shape: Shape,
    pub mental_model: String,
    pub idea: usize,
}

#[derive(Debug)]
pub struct Generated {
    pub questions: Vec<ExamQuestion>,
    pub ideas: IdeasDocument,
}

#[derive(Debug)]
pub struct GeneratedTopic {
    pub ideas: Vec<Idea>,
    pub questions: Vec<ExamQuestion>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, serde::Serialize)]
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
    pub topics: Vec<TopicScore>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TopicScore {
    pub pass: u32,
    pub partial: u32,
    pub total: u32,
    pub passed: bool,
}

impl ExamResult {
    pub fn scored(questions: &[ExamQuestion], grades: Vec<AnswerGrade>, threshold: f64) -> Self {
        let count = questions.iter().map(|q| q.topic + 1).max().unwrap_or(0);
        let mut topics = vec![TopicScore::default(); count];
        for (question, grade) in questions.iter().zip(&grades) {
            let score = &mut topics[question.topic];
            score.total += 1;
            match grade.verdict {
                Verdict::Pass => score.pass += 1,
                Verdict::Partial => score.partial += 1,
                Verdict::Fail => {}
            }
        }
        for score in &mut topics {
            score.passed =
                score.total > 0 && f64::from(score.pass) / f64::from(score.total) >= threshold;
        }
        Self { grades, topics }
    }

    pub fn count(&self) -> TopicScore {
        let mut count = TopicScore::default();
        for score in &self.topics {
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

    pub fn gaps_by_topic(
        &self,
        questions: &[ExamQuestion],
        topics: &[ExamTopic],
    ) -> Vec<(String, Vec<String>)> {
        let mut gaps: Vec<Vec<String>> = vec![Vec::new(); topics.len()];
        for (question, grade) in questions.iter().zip(&self.grades) {
            let Some(topic_gaps) = gaps.get_mut(question.topic) else {
                continue;
            };
            if grade.verdict == Verdict::Pass {
                continue;
            }
            for point in &grade.missed {
                let point = point.trim();
                if !point.is_empty() && !topic_gaps.iter().any(|g| g == point) {
                    topic_gaps.push(point.to_string());
                }
            }
        }
        topics
            .iter()
            .zip(gaps)
            .enumerate()
            .filter(|(index, (_, gaps))| {
                !gaps.is_empty() && self.topics.get(*index).is_some_and(|score| !score.passed)
            })
            .map(|(_, (topic, gaps))| (topic.topic_id.clone(), gaps))
            .collect()
    }
}

pub fn generate_questions(
    deck: &Deck,
    cfg: &ExamConfig,
    ask_cfg: &AskConfig,
) -> Result<Vec<ExamQuestion>> {
    let sources = deck
        .sources()?
        .context("the deck declares no sources to examine against")?;
    let mut ideas = IdeasDocument::default();
    generate_topics(
        &examined_topics(deck),
        &sources,
        &mut ideas,
        cfg,
        ask_cfg,
        &ask::AskJob::default(),
        &mut |_| Ok(()),
    )
}

/// Questions written ahead of a sitting, with the answers given so far.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Prepared {
    pub version: u32,
    pub generated_at_ms: u64,
    pub topics: BTreeMap<String, PreparedTopic>,
    pub questions: Vec<PreparedQuestion>,
    pub answers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedTopic {
    pub fingerprint: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedQuestion {
    pub topic: String,
    pub idea: usize,
    pub prompt: String,
    pub points: Vec<String>,
    pub shape: Shape,
    pub mental_model: String,
}

impl PreparedQuestion {
    fn of(question: &ExamQuestion, topics: &[ExamTopic]) -> Option<Self> {
        Some(Self {
            topic: topics.get(question.topic)?.topic_id.clone(),
            idea: question.idea,
            prompt: question.prompt.clone(),
            points: question.points.clone(),
            shape: question.shape,
            mental_model: question.mental_model.clone(),
        })
    }

    fn at(&self, topic: usize) -> ExamQuestion {
        ExamQuestion {
            prompt: self.prompt.clone(),
            points: self.points.clone(),
            topic,
            shape: self.shape,
            mental_model: self.mental_model.clone(),
            idea: self.idea,
        }
    }
}

impl Prepared {
    pub fn load(path: &Path) -> Result<Option<Self>> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let prepared: Self = serde_json::from_str(&text)
                    .with_context(|| format!("reading {}", path.display()))?;
                if prepared.version != 1 {
                    bail!("unreadable prepared exam {}", path.display());
                }
                Ok(Some(prepared))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self).context("encoding the prepared exam")?;
        crate::exam_ideas::write_atomic(path, json.as_bytes())
    }

    /// This exam narrowed to the questions a sitting kept, with its answers.
    pub fn narrowed(
        &self,
        topics: &[ExamTopic],
        questions: &[ExamQuestion],
        answers: &[String],
    ) -> Self {
        Self {
            questions: questions
                .iter()
                .filter_map(|q| PreparedQuestion::of(q, topics))
                .collect(),
            answers: answers.to_vec(),
            ..self.clone()
        }
    }

    fn fresh(&self, topic_id: &str, fingerprints: &BTreeMap<String, String>) -> bool {
        self.topics
            .get(topic_id)
            .zip(fingerprints.get(topic_id))
            .is_some_and(|(prepared, current)| prepared.fingerprint == *current)
    }

    pub fn is_current(&self, deck: &Deck) -> bool {
        let fingerprints = deck.declared().fingerprints();
        let topic_ids: BTreeSet<String> = deck.topic_ids().into_iter().flatten().collect();
        !self.questions.is_empty()
            && self
                .questions
                .iter()
                .all(|q| topic_ids.contains(&q.topic) && self.fresh(&q.topic, &fingerprints))
            && fingerprints
                .keys()
                .all(|topic_id| self.fresh(topic_id, &fingerprints))
    }

    /// The deck's sourced topics this exam still covers, and its questions
    /// and answers re-indexed onto them; a question whose topic is gone or
    /// whose sources changed is dropped.
    pub fn against(&self, deck: &Deck) -> (Vec<ExamTopic>, Vec<ExamQuestion>, Vec<String>) {
        let fingerprints = deck.declared().fingerprints();
        let topics: Vec<ExamTopic> = examined_topics(deck)
            .into_iter()
            .filter(|topic| {
                self.fresh(&topic.topic_id, &fingerprints)
                    && self.questions.iter().any(|q| q.topic == topic.topic_id)
            })
            .collect();
        let mut questions = Vec::new();
        let mut answers = Vec::new();
        for (q, answer) in self.questions.iter().zip(&self.answers) {
            if let Some(at) = topics.iter().position(|t| t.topic_id == q.topic) {
                questions.push(q.at(at));
                answers.push(answer.clone());
            }
        }
        (topics, questions, answers)
    }
}

pub fn open_for_exam(path: &Path) -> Result<Deck> {
    crate::assemble::stamp_for_session(path)?;
    let mut deck = Deck::load(path).with_context(|| format!("loading {}", path.display()))?;
    deck.cards.retain(|card| card.id().is_some());
    Ok(deck)
}

fn topic_refusal(deck: &Deck, manifest: Option<&crate::sources::Manifest>) -> Option<String> {
    duplicated_topic_error(deck).or_else(|| {
        manifest
            .and_then(|manifest| manifest.validate_bindings(deck).err())
            .map(|error| error.to_string())
    })
}

fn cached_manifest(deck: &Deck) -> Option<&crate::sources::Manifest> {
    deck.declared()
        .manifest
        .as_ref()
        .ok()
        .and_then(Option::as_ref)
}

fn source_boundary_error(deck: &Deck) -> Option<String> {
    (workspace::root_for_deck(&deck.path).is_some() && deck.deck_token.is_some())
        .then(|| crate::assets::validate_member(deck).err())
        .flatten()
        .map(|error| format!("{error:#}"))
}

/// The shared findings text when a topic id of this deck is on two topics;
/// nothing attaches to such an id, so nothing is examined.
pub fn duplicated_topic_error(deck: &Deck) -> Option<String> {
    let root = workspace::root_for_deck(&deck.path);
    let findings = crate::topic::duplicated_topics(deck, root.as_deref());
    (!findings.is_empty()).then(|| {
        findings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    })
}

pub fn generate_topic(
    topic: &ExamTopic,
    sources: &[SourceText],
    ideas: Option<&IdeaTopic>,
    cfg: &ExamConfig,
    ask_cfg: &AskConfig,
    job: &ask::AskJob,
) -> Result<GeneratedTopic> {
    let run = run_config(cfg, ask_cfg);
    let listed: Vec<Idea> = match ideas.filter(|known| !known.ideas.is_empty()) {
        Some(known) => known.ideas.clone(),
        None => with_one_retry(
            job,
            &run,
            &ideas_prompt(topic, sources, cfg),
            checked_ideas,
            "the listed ideas",
        )?
        .into_iter()
        .map(|text| Idea {
            text,
            ..Idea::default()
        })
        .collect(),
    };
    let chosen = crate::exam_ideas::select_ideas(&listed);
    let chosen_text: Vec<String> = chosen.iter().map(|&at| listed[at].text.clone()).collect();
    let mut questions = with_one_retry(
        job,
        &run,
        &questions_prompt(topic, sources, &chosen_text, cfg),
        |raw| checked_questions(&chosen_text, raw),
        "the generated questions",
    )?;
    for question in &mut questions {
        question.idea = chosen[question.idea];
    }
    Ok(GeneratedTopic {
        ideas: listed,
        questions,
    })
}

/// One call per topic (two when its ideas are listed first); each question's
/// `topic` indexes `topics`.
pub fn generate_topics(
    topics: &[ExamTopic],
    sources: &DeckSources,
    ideas: &mut IdeasDocument,
    cfg: &ExamConfig,
    ask_cfg: &AskConfig,
    job: &ask::AskJob,
    after_topic: &mut dyn FnMut(&IdeasDocument) -> Result<()>,
) -> Result<Vec<ExamQuestion>> {
    let fingerprints = sources.manifest.fingerprints()?;
    ideas
        .topics
        .retain(|topic_id, _| fingerprints.contains_key(topic_id));
    let mut questions = Vec::new();
    for (at, topic) in topics.iter().enumerate() {
        let Some(fingerprint) = fingerprints.get(&topic.topic_id) else {
            bail!("topic `{}` has no sources to examine against", topic.label);
        };
        let texts = sources.texts_of(&topic.topic_id)?;
        let entry = ideas.for_topic(&topic.topic_id, fingerprint);
        let generated = generate_topic(topic, &texts, Some(&*entry), cfg, ask_cfg, job)?;
        entry.ideas = generated.ideas;
        after_topic(ideas)?;
        questions.extend(
            generated
                .questions
                .into_iter()
                .map(|question| ExamQuestion {
                    topic: at,
                    ..question
                }),
        );
    }
    Ok(questions)
}

fn with_one_retry<T>(
    job: &ask::AskJob,
    run: &AskConfig,
    prompt: &str,
    check: impl Fn(&str) -> std::result::Result<T, String>,
    what: &str,
) -> Result<T> {
    let reason = match check(&ask::run_in(job, run, prompt)?) {
        Ok(value) => return Ok(value),
        Err(reason) => reason,
    };
    let retry = format!(
        "{prompt}\n\nYour previous answer was rejected: {reason}. Write the whole answer \
         again and follow every rule above."
    );
    match check(&ask::run_in(job, run, &retry)?) {
        Ok(value) => Ok(value),
        Err(reason) => bail!("{what} were rejected twice: {reason}"),
    }
}

pub(crate) fn checked_ideas(raw: &str) -> std::result::Result<Vec<String>, String> {
    let dto: IdeasDto =
        parse_json(raw).map_err(|error| format!("it was not the JSON asked for ({error:#})"))?;
    let mut ideas: Vec<String> = Vec::new();
    for idea in dto.ideas.iter().map(|idea| idea.trim()) {
        if !idea.is_empty() && !ideas.iter().any(|known| known == idea) {
            ideas.push(idea.to_string());
        }
    }
    if ideas.is_empty() {
        return Err("it lists no idea".to_string());
    }
    Ok(ideas)
}

pub(crate) fn checked_questions(
    ideas: &[String],
    raw: &str,
) -> std::result::Result<Vec<ExamQuestion>, String> {
    let dto: QuestionsDto =
        parse_json(raw).map_err(|error| format!("it was not the JSON asked for ({error:#})"))?;
    let mut covered = vec![false; ideas.len()];
    let mut questions = Vec::new();
    for generated in dto.questions {
        if generated.prompt.trim().is_empty()
            || generated.points.is_empty()
            || generated.points.iter().any(|point| point.trim().is_empty())
        {
            return Err("a question has an empty prompt or no key points".to_string());
        }
        let Some(which) = generated
            .idea
            .checked_sub(1)
            .filter(|which| *which < ideas.len())
        else {
            return Err(format!(
                "a question names idea {}, but the topic lists {}",
                generated.idea,
                ideas.len()
            ));
        };
        if covered[which] {
            return Err(format!("idea {} is asked more than once", which + 1));
        }
        covered[which] = true;
        questions.push(ExamQuestion {
            prompt: generated.prompt,
            points: generated.points,
            topic: 0,
            shape: generated.shape,
            mental_model: ideas[which].clone(),
            idea: which,
        });
    }
    if let Some(which) = covered.iter().position(|done| !done) {
        return Err(format!(
            "the idea `{}` is listed but no question asks about it",
            ideas[which]
        ));
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

#[cfg(test)]
static GENERATION_SPAWNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn spawn_generation(
    topics: Vec<ExamTopic>,
    sources: DeckSources,
    ideas: Option<IdeasDocument>,
    cfg: ExamConfig,
    ask_cfg: AskConfig,
) -> (Receiver<Result<Generated, String>>, ask::AskJob) {
    #[cfg(test)]
    GENERATION_SPAWNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let (tx, rx) = channel();
    let job = ask::AskJob::default();
    let worker = job.clone();
    std::thread::spawn(move || {
        let mut ideas = ideas.unwrap_or_default();
        let r = generate_topics(
            &topics,
            &sources,
            &mut ideas,
            &cfg,
            &ask_cfg,
            &worker,
            &mut |_| Ok(()),
        )
        .map(|questions| Generated { questions, ideas })
        .map_err(|e| format!("{e:#}"));
        let _ = tx.send(r);
    });
    (rx, job)
}

/// Where a deck's exam files live: its store root's `.alix/exam/`.
pub fn exam_files(deck: &Deck) -> Option<(PathBuf, PathBuf)> {
    let token = deck.deck_token.as_deref()?;
    let files = crate::state::UserFiles::new(workspace::content_root(&deck.path));
    Some((files.exam_ideas_for(token), files.exam_prepared_for(token)))
}

/// Whether the deck holds a prepared exam written from its current sources.
pub fn has_current_prepared(deck: &Deck) -> bool {
    exam_files(deck)
        .and_then(|(_, path)| Prepared::load(&path).ok().flatten())
        .is_some_and(|prepared| prepared.is_current(deck))
}

/// Writes a deck's questions ahead of its sitting, in the background: the
/// prepared exam and the updated idea list land in the deck's store before
/// the receiver reports, so a learner who left still finds them. Questions of
/// an earlier prepared exam whose topic is still asked over the same sources
/// are kept with their answers; every other topic is written afresh.
pub fn spawn_prepare(
    deck: &Deck,
    topics: Vec<ExamTopic>,
    cfg: ExamConfig,
    ask_cfg: AskConfig,
) -> (Receiver<Result<(), String>>, ask::AskJob) {
    let (tx, rx) = channel();
    let job = ask::AskJob::default();
    if let Some(error) = topic_refusal(deck, cached_manifest(deck)) {
        let _ = tx.send(Err(error));
        return (rx, job);
    }
    let worker = job.clone();
    let root = workspace::root_for_deck(&deck.path);
    let deck_id = deck.deck_token.clone();
    let files = exam_files(deck);
    std::thread::spawn(move || {
        let r = (|| -> Result<()> {
            let (ideas_path, prepared_path) =
                files.context("the deck has no id to keep its exam under")?;
            let sources = match (root, deck_id) {
                (Some(root), Some(deck_id)) => crate::sources::load_verified(&root, &deck_id)?,
                _ => None,
            }
            .context("the deck declares no sources to examine against")?;
            let fingerprints = sources.manifest.fingerprints()?;
            let mut ideas = IdeasDocument::load(&ideas_path)?.unwrap_or_default();
            let earlier = Prepared::load(&prepared_path).ok().flatten();
            let kept = |topic: &ExamTopic| {
                earlier.as_ref().is_some_and(|prepared| {
                    prepared.fresh(&topic.topic_id, &fingerprints)
                        && prepared.questions.iter().any(|q| q.topic == topic.topic_id)
                })
            };
            let stale: Vec<ExamTopic> = topics.iter().filter(|t| !kept(t)).cloned().collect();
            let written = generate_topics(
                &stale,
                &sources,
                &mut ideas,
                &cfg,
                &ask_cfg,
                &worker,
                &mut |ideas| ideas.save(&ideas_path),
            )?;
            ideas.save(&ideas_path)?;
            let run = run_config(&cfg, &ask_cfg);
            let mut prepared = Prepared {
                version: 1,
                generated_at_ms: crate::time::now_ms(),
                topics: BTreeMap::new(),
                questions: Vec::new(),
                answers: Vec::new(),
            };
            for topic in &topics {
                let (questions, answers, entry): (Vec<PreparedQuestion>, Vec<String>, _) =
                    match earlier.as_ref().filter(|_| kept(topic)) {
                        Some(earlier) => {
                            let (questions, answers) = earlier
                                .questions
                                .iter()
                                .zip(&earlier.answers)
                                .filter(|(q, _)| q.topic == topic.topic_id)
                                .map(|(q, a)| (q.clone(), a.clone()))
                                .unzip();
                            let entry = earlier
                                .topics
                                .get(&topic.topic_id)
                                .cloned()
                                .context("a kept topic has no prepared entry")?;
                            (questions, answers, entry)
                        }
                        None => {
                            let at = stale
                                .iter()
                                .position(|t| t.topic_id == topic.topic_id)
                                .context("a topic to write was skipped")?;
                            let questions: Vec<PreparedQuestion> = written
                                .iter()
                                .filter(|q| q.topic == at)
                                .filter_map(|q| PreparedQuestion::of(q, &stale))
                                .collect();
                            let answers = vec![String::new(); questions.len()];
                            let entry = PreparedTopic {
                                fingerprint: fingerprints
                                    .get(&topic.topic_id)
                                    .cloned()
                                    .unwrap_or_default(),
                                model: run.model.clone(),
                                effort: run.effort.clone(),
                            };
                            (questions, answers, entry)
                        }
                    };
                prepared.topics.insert(topic.topic_id.clone(), entry);
                prepared.questions.extend(questions);
                prepared.answers.extend(answers);
            }
            for (topic_id, fingerprint) in &fingerprints {
                prepared
                    .topics
                    .entry(topic_id.clone())
                    .or_insert_with(|| PreparedTopic {
                        fingerprint: fingerprint.clone(),
                        model: run.model.clone(),
                        effort: run.effort.clone(),
                    });
            }
            prepared.save(&prepared_path)
        })()
        .map_err(|e| format!("{e:#}"));
        let _ = tx.send(r);
    });
    (rx, job)
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
    gaps_by_topic: Vec<(String, Vec<String>)>,
    cfg: ExamConfig,
    ask_cfg: AskConfig,
) -> Receiver<Result<Vec<TopicCards>, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let r = gaps_by_topic
            .iter()
            .map(|(topic_id, gaps)| {
                Ok(TopicCards {
                    topic_id: topic_id.clone(),
                    cards: remediation_cards(gaps, &cfg, &ask_cfg)?,
                })
            })
            .collect::<Result<Vec<_>>>()
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(r);
    });
    rx
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopicCards {
    pub topic_id: String,
    pub cards: String,
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
    RemediationCards(Vec<TopicCards>),
}

enum Pending {
    Questions(Receiver<Result<Generated, String>>),
    Grade(Receiver<Result<ExamResult, String>>),
    Remediation(Receiver<Result<Vec<TopicCards>, String>>),
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
    remediated_topics: HashSet<String>,
    topics: Vec<ExamTopic>,
    deck_topics: Vec<ExamTopic>,
    start: SittingStart,
    unlocked: bool,
    ideas: Option<IdeasDocument>,
    locked: Vec<bool>,
    job: ask::AskJob,
}

impl Drop for Sitting {
    fn drop(&mut self) {
        self.job.cancel();
    }
}

fn sitting_record(
    topics: &[ExamTopic],
    full: bool,
    result: &ExamResult,
) -> crate::store::SittingRecord {
    let count = result.count();
    crate::store::SittingRecord {
        topics: topics
            .iter()
            .zip(&result.topics)
            .filter(|(_, score)| score.total > 0)
            .map(|(topic, score)| crate::store::TopicRecord {
                topic_id: topic.topic_id.clone(),
                fingerprint: topic.fingerprint.clone(),
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

    pub fn start(
        deck: &Deck,
        start: SittingStart,
        strictness: Strictness,
        cfg: ExamConfig,
        ask_cfg: AskConfig,
    ) -> Self {
        match deck.sources() {
            Ok(Some(sources)) => Self::start_with(
                deck,
                examined_topics(deck),
                sources,
                None,
                start,
                strictness,
                cfg,
                ask_cfg,
            ),
            Ok(None) => Self::refused(
                deck,
                "the deck declares no sources to examine against".to_string(),
                strictness,
                cfg,
                ask_cfg,
            ),
            Err(error) => Self::refused(deck, format!("{error}"), strictness, cfg, ask_cfg),
        }
    }

    fn refused(
        deck: &Deck,
        error: String,
        strictness: Strictness,
        cfg: ExamConfig,
        ask_cfg: AskConfig,
    ) -> Self {
        let mut sitting = Self::idle(
            deck,
            Vec::new(),
            SittingStart::default(),
            strictness,
            cfg,
            ask_cfg,
        );
        sitting.error = Some(error);
        sitting.phase = Phase::Results;
        sitting
    }

    /// A sitting over `topics`, written from `sources`: only topics the
    /// sources bind are sent to the model.
    #[expect(
        clippy::too_many_arguments,
        reason = "a sitting's whole starting state"
    )]
    pub fn start_with(
        deck: &Deck,
        topics: Vec<ExamTopic>,
        sources: DeckSources,
        ideas: Option<IdeasDocument>,
        start: SittingStart,
        strictness: Strictness,
        cfg: ExamConfig,
        ask_cfg: AskConfig,
    ) -> Self {
        let sourced = sources.sourced_topics();
        let topics: Vec<ExamTopic> = topics
            .into_iter()
            .filter(|topic| sourced.contains(&topic.topic_id))
            .collect();
        let refusal = topic_refusal(deck, Some(&sources.manifest))
            .or_else(|| source_boundary_error(deck))
            .or_else(|| {
                topics
                    .is_empty()
                    .then(|| "the deck has no topic with sources to examine".to_string())
            });
        if let Some(error) = refusal {
            return Self::refused(deck, error, strictness, cfg, ask_cfg);
        }
        let mut sitting = Self::idle(deck, topics, start, strictness, cfg, ask_cfg);
        let (rx, job) = spawn_generation(
            sitting.topics.clone(),
            sources,
            ideas,
            sitting.cfg.clone(),
            sitting.ask_cfg.clone(),
        );
        sitting.job = job;
        sitting.pending = Some(Pending::Questions(rx));
        sitting.pending_since = Some(crate::time::now_ms());
        sitting.phase = Phase::Generating;
        sitting
    }

    fn idle(
        deck: &Deck,
        topics: Vec<ExamTopic>,
        start: SittingStart,
        strictness: Strictness,
        cfg: ExamConfig,
        ask_cfg: AskConfig,
    ) -> Self {
        Self {
            subject: deck.subject.clone(),
            deck_token: deck.deck_token.clone().unwrap_or_default(),
            deck_path: Some(deck.path.clone()),
            deck_fingerprints: deck.cards.iter().map(|c| c.block_fingerprint).collect(),
            strictness,
            cfg,
            ask_cfg,
            phase: Phase::Answering,
            questions: Vec::new(),
            answers: Vec::new(),
            current: 0,
            result: None,
            pending: None,
            error: None,
            pending_since: None,
            remediated_count: None,
            remediated_topics: HashSet::new(),
            topics,
            deck_topics: examined_topics(deck),
            start,
            unlocked: false,
            ideas: None,
            locked: Vec::new(),
            job: ask::AskJob::default(),
        }
    }

    /// A sitting over questions generated earlier: it opens in the answering
    /// phase at the first unanswered question, with every answered one locked.
    #[expect(
        clippy::too_many_arguments,
        reason = "a sitting's whole resumable state"
    )]
    pub fn from_prepared(
        deck: &Deck,
        topics: Vec<ExamTopic>,
        questions: Vec<ExamQuestion>,
        answers: Vec<String>,
        ideas: crate::exam_ideas::IdeasDocument,
        start: SittingStart,
        strictness: Strictness,
        cfg: ExamConfig,
        ask_cfg: AskConfig,
    ) -> Self {
        if let Some(error) = topic_refusal(deck, cached_manifest(deck)) {
            return Self::refused(deck, error, strictness, cfg, ask_cfg);
        }
        let mut sitting = Self::idle(deck, topics, start, strictness, cfg, ask_cfg);
        sitting.locked = answers.iter().map(|a| !a.trim().is_empty()).collect();
        sitting.current = sitting.locked.iter().position(|l| !l).unwrap_or(0);
        sitting.answers = answers;
        sitting.questions = questions;
        sitting.ideas = Some(ideas);
        sitting
    }

    pub fn ideas(&self) -> Option<&crate::exam_ideas::IdeasDocument> {
        self.ideas.as_ref()
    }

    pub fn is_locked(&self, at: usize) -> bool {
        self.locked.get(at).copied().unwrap_or(false)
    }

    fn lock_current(&mut self) {
        let at = self.current;
        if self.answers.get(at).is_some_and(|a| !a.trim().is_empty())
            && let Some(slot) = self.locked.get_mut(at)
        {
            *slot = true;
        }
    }

    pub fn topics(&self) -> &[ExamTopic] {
        &self.topics
    }

    pub fn full(&self) -> bool {
        self.topics.len() == self.deck_topics.len()
    }

    pub fn unlocked(&self) -> bool {
        self.unlocked
    }

    pub fn eligible(&self) -> bool {
        self.start.eligible
    }

    pub fn stamp(&self) -> &crate::store::ExamStamp {
        &self.start.stamp
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
            && !self.is_locked(self.current)
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
        self.lock_current();
        if self.current + 1 < self.questions.len() {
            self.current += 1;
        }
    }
    pub fn prev(&mut self) {
        self.lock_current();
        self.current = self.current.saturating_sub(1);
    }
    pub fn goto(&mut self, i: usize) {
        self.lock_current();
        if i < self.questions.len() {
            self.current = i;
        }
    }

    fn accept_questions(&mut self, generated: Generated) {
        self.answers = vec![String::new(); generated.questions.len()];
        self.locked = vec![false; generated.questions.len()];
        self.questions = generated.questions;
        self.ideas = Some(generated.ideas);
        self.current = 0;
        self.phase = Phase::Answering;
    }

    fn record_ideas(&mut self, result: &ExamResult) {
        let asked: Vec<crate::exam_ideas::AskedIdea> = self
            .questions
            .iter()
            .zip(&result.grades)
            .filter_map(|(question, grade)| {
                Some(crate::exam_ideas::AskedIdea {
                    topic: self.topics.get(question.topic)?.topic_id.clone(),
                    idea: question.idea,
                    passed: grade.verdict == Verdict::Pass,
                })
            })
            .collect();
        if let Some(ideas) = self.ideas.as_mut() {
            ideas.record(&asked, crate::time::now_ms());
        }
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
        self.phase == Phase::Results && !self.gaps_by_topic().is_empty()
    }

    fn gaps_by_topic(&self) -> Vec<(String, Vec<String>)> {
        self.result
            .as_ref()
            .map(|result| result.gaps_by_topic(&self.questions, &self.topics))
            .unwrap_or_default()
            .into_iter()
            .filter(|(topic_id, _)| !self.remediated_topics.contains(topic_id))
            .collect()
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
            self.gaps_by_topic(),
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
                self.record_ideas(&result);
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
                    let record = sitting_record(&self.topics, self.full(), result);
                    self.unlocked = store.record_sitting(
                        &self.deck_token,
                        &self.deck_topics,
                        &record,
                        now_ms,
                        self.start.eligible,
                        &self.start.stamp,
                    );
                }
                store_mutated = true;
            }
            Some(Effect::RemediationCards(by_topic)) => {
                store_mutated = true;
                let mut stored = 0;
                for topic in &by_topic {
                    match crate::store::store_remediation_cards(
                        store,
                        self.deck_path.as_deref(),
                        &self.deck_token,
                        &topic.topic_id,
                        &self.deck_fingerprints,
                        &topic.cards,
                        retire_after_days,
                    ) {
                        Ok(n) => {
                            stored += n;
                            self.remediated_topics.insert(topic.topic_id.clone());
                        }
                        Err(e) => {
                            self.error = Some(format!("{e}"));
                            self.phase = Phase::Results;
                            return ExamPollOutcome {
                                advanced,
                                store_mutated,
                            };
                        }
                    }
                }
                self.remediated_count = Some(stored);
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
    Questions(Result<Generated, String>),
    Grade(Result<ExamResult, String>),
    Remediation(Result<Vec<TopicCards>, String>),
}

fn thread_gone() -> String {
    "the exam helper exited unexpectedly".to_string()
}

/// A rough wait for one generation, shown while it runs; never a commitment.
pub fn wait_estimate(ask_cfg: &AskConfig) -> Option<&'static str> {
    (ask_cfg.backend == crate::config::BackendKind::Claude).then_some("usually 10 to 20 minutes")
}

pub(crate) fn run_config(cfg: &ExamConfig, ask_cfg: &AskConfig) -> AskConfig {
    AskConfig {
        model: cfg.model.clone().or_else(|| ask_cfg.model.clone()),
        effort: cfg.effort.clone().or_else(|| ask_cfg.effort.clone()),
        timeout_secs: cfg.timeout_secs,
        cwd: None,
        source_access: false,
        ..ask_cfg.clone()
    }
}

pub fn unreadable_exam_entries(deck: &Deck) -> Vec<(String, String)> {
    let root = workspace::content_root(&deck.path);
    deck.exam
        .iter()
        .filter(|src| !deck::is_url(src))
        .filter_map(|src| {
            let path = crate::source::source_path(src, Some(&root))?;
            std::fs::read_to_string(&path)
                .with_context(|| format!("cannot read `exam:` {}", path.display()))
                .err()
                .map(|error| (src.trim().to_string(), format!("{error:#}")))
        })
        .collect()
}

fn source_section(sources: &[SourceText]) -> String {
    let mut out = String::new();
    for source in sources {
        out.push_str(&format!("\nSource `{}`:\n<<<SOURCE\n", source.display));
        out.push_str(&source.text);
        if !source.text.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("SOURCE\n");
    }
    out
}

fn topic_scope(topic: &ExamTopic) -> String {
    let mut scope = format!("Topic: {}\n", topic.label);
    for line in topic.context.iter().skip(1) {
        scope.push_str(line);
        scope.push('\n');
    }
    scope.push_str("Cards:\n");
    for card in &topic.cards {
        scope.push_str(&format!("- {}\n", card.replace('\n', "\n  ")));
    }
    scope.push('\n');
    scope
}

pub fn ideas_prompt(topic: &ExamTopic, sources: &[SourceText], cfg: &ExamConfig) -> String {
    let sources = source_section(sources);
    let scope = topic_scope(topic);
    let mut prompt = format!(
        "You are preparing an oral exam that verifies a student truly UNDERSTANDS a \
         topic.\n\n\
         {sources}\n\
         The student drilled flashcards on this material. The topic below is marked out \
         by its heading, with the prose under it when there is any. Its cards show only \
         what the student has drilled so far; they do not mark the edge of the topic.\n\n\
         {scope}\
         List the topic's ideas: the distinct ideas a student must understand to master \
         this topic, as the sources above teach them. Cover everything the sources teach \
         within the topic, whether or not a card states it. List as many as the topic \
         really holds, at least one; name each idea in a short phrase.\n\n\
         Output ONLY JSON in exactly this shape, no prose, no code fences:\n\
         {{\"ideas\": [\"...\", \"...\"]}}"
    );
    push_extra(&mut prompt, cfg);
    prompt
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

pub fn questions_prompt(
    topic: &ExamTopic,
    sources: &[SourceText],
    ideas: &[String],
    cfg: &ExamConfig,
) -> String {
    let sources = source_section(sources);
    let mut scope = String::from(
        "The student drilled flashcards on this material. The topic's cards are SCOPE \
         ONLY: they show what the student already drilled. NEVER ask what a card states, \
         and never rephrase a card; test the mental model behind the topic instead.\n\n",
    );
    scope.push_str(&topic_scope(topic));
    scope.push_str("The ideas this exam examines, numbered:\n");
    for (n, idea) in ideas.iter().enumerate() {
        scope.push_str(&format!("{}. {idea}\n", n + 1));
    }
    scope.push('\n');
    let ask = format!(
        "{scope}Write EXACTLY one question for every idea listed, no more. An idea a \
         card states is still examined: ask the student to apply it, never to recite the \
         card. Start with an instance and then take the three shapes in turn:\n\
         - instance: a concrete case the sources support but no card shows \
         (\"given this program, what does the compiler report?\");\n\
         - boundary: when does the topic's idea stop holding, or what changes \
         if a condition flips;\n\
         - fix: a short statement or snippet with exactly one seeded mistake about \
         the topic's idea; the student must find and correct it.\n\
         Every question must be answerable from the sources above and demand \
         reasoning in the student's own words. Tag every question with its shape and \
         the number of the idea it tests.\n\n"
    );
    let shape = "{\"questions\": [{\"shape\": \"instance|boundary|fix\", \
                 \"idea\": 1, \"prompt\": \"...\", \
                 \"points\": [\"...\", \"...\"]}]}";
    let mut prompt = format!(
        "You are a strict examiner writing an oral exam that verifies a student \
         truly UNDERSTANDS a topic, not whether they memorized isolated facts.\n\n\
         {sources}\n\
         {ask}\
         {QUESTION_FORM}\
         For each question, list the key points a correct answer MUST contain: \
         the specific, source-grounded facts or reasoning steps you will grade \
         against. Be concrete; these are the rubric.\n\n\
         Output ONLY JSON in exactly this shape, no prose, no code fences around \
         it (a fence inside a `prompt` string is fine):\n\
         {shape}"
    );
    push_extra(&mut prompt, cfg);
    prompt
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
/// depth; a topic heading alone is prose.
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
#[serde(deny_unknown_fields)]
struct IdeasDto {
    ideas: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuestionsDto {
    questions: Vec<GeneratedQuestionDto>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneratedQuestionDto {
    shape: Shape,
    idea: usize,
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

    /// Generated cards are recognized at every card depth: a check that
    /// only knows `## ` would reject a valid generated sub-card set as
    /// empty, or keep prose above the first front.
    #[test]
    fn generated_card_text_is_recognized_at_every_depth() {
        assert!(has_card_fronts("### only a sub-card\nanswer\n"));
        assert!(has_card_fronts("#### deep\nanswer\n"));
        assert!(!has_card_fronts("# just a topic\nprose\n"));
    }

    /// A fact-deck exam grades against the source, never a card's topic
    /// context. This is a signature-name tripwire for that boundary, not a
    /// generic ban on deck-derived values: question generation takes the
    /// topic as scope.
    #[test]
    fn exam_prompt_signatures_ban_topic_like_names() {
        let source = include_str!("exam.rs");
        let builders: [(&str, &[&str]); 4] = [
            ("fn questions_prompt(", &["topic: &ExamTopic"]),
            ("fn ideas_prompt(", &["topic: &ExamTopic"]),
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
            for banned in [
                "context",
                "section",
                "Section",
                "topic",
                "Topic",
                "cards",
                "deck_body",
            ] {
                assert!(
                    !unpinned.contains(banned),
                    "{builder} takes `{banned}`: topic/card context must not enter \
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
    fn questions_prompt_carries_source_strictness_and_json_shape() {
        let p = questions_prompt(
            &one_topic(),
            &one_source(),
            &["m".to_string()],
            &ExamConfig::default(),
        );
        assert!(p.contains("Source `sdk:src/lib.rs`:\n<<<SOURCE\nfn main() {}\nSOURCE\n"));
        assert!(p.contains("strict examiner"));
        assert!(p.contains("\"questions\""));
    }

    #[test]
    fn questions_prompt_asks_one_topic_in_the_three_shapes_with_cards_as_scope_only() {
        let topic = ExamTopic {
            label: "Moves".to_string(),
            context: vec![
                "Moves".to_string(),
                "Values move on assignment.".to_string(),
            ],
            cards: vec!["Q: What does `let b = a;` do to a String?\nA: Moves it".to_string()],
            ..one_topic()
        };
        let p = questions_prompt(
            &topic,
            &one_source(),
            &["moves".to_string(), "drops".to_string()],
            &ExamConfig::default(),
        );
        assert!(p.contains("Topic: Moves\n"), "topic heading: {p}");
        assert!(p.contains("Values move on assignment."), "topic prose: {p}");
        assert!(
            p.contains("What does `let b = a;` do to a String?"),
            "card scope: {p}"
        );
        assert!(
            p.contains("NEVER ask what a card states"),
            "scope only: {p}"
        );
        for rule in [
            "The ideas this exam examines, numbered:\n1. moves\n2. drops\n",
            "Write EXACTLY one question for every idea listed, no more",
            "ask the student to apply it, never to recite the \
         card",
            "the number of the idea it tests",
        ] {
            assert!(p.contains(rule), "ideas rule {rule:?}: {p}");
        }
        for shape in ["instance:", "boundary:", "fix:"] {
            assert!(p.contains(shape), "shape {shape}: {p}");
        }
        for tag in ["\"shape\"", "\"idea\"", "\"prompt\"", "\"points\""] {
            assert!(p.contains(tag), "json carries {tag}: {p}");
        }
        assert!(
            !p.contains("\"topic\""),
            "one topic per call names none: {p}"
        );
    }

    #[test]
    fn topic_status_follows_the_topics_counting_result() {
        use TopicStatus::*;
        type Row = (
            &'static str,
            [Option<(bool, bool)>; 2],
            [TopicStatus; 2],
            ExamFilter,
        );
        // per topic: (pass, recorded on the current fingerprint)
        let rows: [Row; 6] = [
            (
                "never examined",
                [None, None],
                [Unexamined, Unexamined],
                ExamFilter::All,
            ),
            (
                "all passed",
                [Some((true, true)), Some((true, true))],
                [Passed, Passed],
                ExamFilter::None,
            ),
            (
                "a fail beside a pass",
                [Some((false, true)), Some((true, true))],
                [NeedsWork, Passed],
                ExamFilter::Partial,
            ),
            (
                "one topic unexamined",
                [Some((true, true)), None],
                [Passed, Unexamined],
                ExamFilter::Partial,
            ),
            (
                "a pass on older material reads unexamined",
                [Some((true, false)), Some((true, true))],
                [Unexamined, Passed],
                ExamFilter::Partial,
            ),
            (
                "every topic needs work",
                [Some((false, true)), Some((false, true))],
                [NeedsWork, NeedsWork],
                ExamFilter::All,
            ),
        ];
        for (label, records, expected, filter) in rows {
            let ws = workspace_with(
                "# A\n<!-- id: topic-a -->\n\n## a1\nx\n<!-- id: card-a1 -->\n\n## a2\nx\n\
                 <!-- id: card-a2 -->\n\n# B\n<!-- id: topic-b -->\n\n## b1\nx\n<!-- id: card-b1 -->\n",
            );
            source(&ws, "s.txt", "s\n", &["topic-a", "topic-b"]);
            let deck = Deck::load(ws.deck()).unwrap();
            let mut store = Store::open(ws.root().join("p.json")).unwrap();
            let topics = examined_topics(&deck);
            let record = crate::store::SittingRecord {
                topics: topics
                    .iter()
                    .zip(records)
                    .filter_map(|(topic, record)| {
                        record.map(|(pass, current)| crate::store::TopicRecord {
                            topic_id: topic.topic_id.clone(),
                            fingerprint: if current {
                                topic.fingerprint.clone()
                            } else {
                                "older".to_string()
                            },
                            pass,
                        })
                    })
                    .collect(),
                ..Default::default()
            };
            store.record_sitting(DECK, &topics, &record, 1, false, &Default::default());
            let status = deck.exam_status(&store);
            assert_eq!(expected.to_vec(), status.topics, "{label}: statuses");
            assert_eq!(filter, status.filter(), "{label}: filter");
            let tally = status.tally();
            assert_eq!(
                2,
                tally.passed + tally.needs_work + tally.unexamined,
                "{label}: the tally counts every topic once"
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
    fn topics_follow_headings_keep_repeats_and_label_the_leading_topic_with_the_deck_title() {
        let rows = [
            (
                "no heading",
                "## q1?\na\n\n## q2?\nb\n",
                vec![("Ownership", 2)],
            ),
            (
                "two headings",
                "# A\n\n## a1?\none\n\n# B\n\n## b1?\ntwo\n",
                vec![("Ownership", 0), ("A", 1), ("B", 1)],
            ),
            (
                "repeated heading",
                "# A\n\n## a1?\none\n\n# B\n\n## b1?\ntwo\n\n# A\n\n## a2?\nthree\n",
                vec![("Ownership", 0), ("A", 1), ("B", 1), ("A", 1)],
            ),
            (
                "a heading with prose and no cards",
                "## q?\na\n\n# Empty\n\nJust prose.\n\n# B\n\n## b?\nc\n",
                vec![("Ownership", 1), ("Empty", 0), ("B", 1)],
            ),
        ];
        for (label, body, expected) in rows {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("d.md");
            std::fs::write(&path, format!("---\ntitle: Ownership\n---\n\n{body}")).unwrap();
            crate::stamp::stamp_deck(&path).unwrap();
            let deck = Deck::load(&path).unwrap();
            let topics = exam_topics(&deck, None);
            let got: Vec<(&str, usize)> = topics
                .iter()
                .map(|s| (s.label.as_str(), s.card_ids.len()))
                .collect();
            assert_eq!(expected, got, "{label}: (label, member ids) per topic");
            assert_eq!(
                deck.deck_token.as_deref(),
                Some(topics[0].topic_id.as_str()),
                "{label}: the leading topic is the deck id"
            );
            let mut every_id: Vec<String> =
                deck.cards.iter().filter_map(|card| card.id()).collect();
            let mut listed: Vec<String> = topics.iter().flat_map(|s| s.card_ids.clone()).collect();
            every_id.sort();
            listed.sort();
            assert_eq!(
                every_id, listed,
                "{label}: every card id belongs to exactly one topic"
            );
        }
    }

    #[test]
    fn questions_prompt_asks_for_standalone_fenced_questions_that_hide_the_answer() {
        let p = questions_prompt(
            &one_topic(),
            &one_source(),
            &["m".to_string()],
            &ExamConfig::default(),
        );
        for rule in [
            "It stands alone.",
            "Code goes in a fenced block",
            "one statement per line",
            "no thinking aloud and no self-correction",
            "Never give the answer away.",
            "a fence inside a `prompt` string is fine",
        ] {
            assert!(p.contains(rule), "missing {rule:?}");
        }
    }

    #[test]
    fn ideas_prompt_bounds_the_topic_by_the_heading_and_shows_cards_only_as_what_is_drilled() {
        let p = ideas_prompt(&one_topic(), &one_source(), &ExamConfig::default());
        assert!(
            p.contains("marked out by its heading, with the prose under it when there is any"),
            "the heading bounds the topic: {p}"
        );
        assert!(
            p.contains(
                "cards show only what the student has drilled so far; they do not mark the edge"
            ),
            "the cards are not the topic's boundary: {p}"
        );
        assert!(
            p.contains("Source `sdk:src/lib.rs`"),
            "the topic's sources: {p}"
        );
        assert!(p.contains("{\"ideas\": ["), "the reply shape: {p}");
    }

    #[test]
    fn questions_prompt_honors_extra() {
        let cfg = ExamConfig {
            extra: Some("Focus on lifetimes.".to_string()),
            ..ExamConfig::default()
        };
        let p = questions_prompt(&one_topic(), &one_source(), &["m".to_string()], &cfg);
        assert!(p.contains("Additional instructions:"));
        assert!(p.contains("Focus on lifetimes."));
    }

    #[test]
    fn exam_extra_reaches_the_idea_listing_call_that_decides_coverage() {
        let _lock = exec_lock();
        let ws = workspace_with("## Why move?\nownership\n");
        source(&ws, "moves.txt", "Moves transfer ownership.\n", &[]);
        let dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(dir.path(), &[IDEAS, ONE_QUESTION]);
        let cfg = ExamConfig {
            extra: Some("Focus on lifetimes.".to_string()),
            ..ExamConfig::default()
        };
        generate_questions(&opened(&ws), &cfg, &ask_config(&cli)).unwrap();
        let first = std::fs::read_to_string(dir.path().join("prompt-1")).unwrap();
        assert!(
            first.contains("Focus on lifetimes.") && first.contains("\"ideas\""),
            "the first call fixes the allowed idea list, so exam guidance must reach it: {first}"
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
        let raw = "Here you go:\n```json\n{\"questions\": [{\"shape\": \"fix\", \"idea\": 1, \
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
    fn topic_pass_and_sitting_count_follow_the_verdicts() {
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
                "a PARTIAL fails its topic at 1.0",
                &[(0, Pass), (0, Partial), (1, Pass)],
                1.0,
                [false, true],
                (2, 1, 3),
                false,
            ),
            (
                "a PARTIAL passes its topic at 0.5",
                &[(0, Pass), (0, Partial), (1, Pass)],
                0.5,
                [true, true],
                (2, 1, 3),
                true,
            ),
            (
                "a fail in one topic leaves the other passed",
                &[(0, Fail), (1, Pass)],
                1.0,
                [false, true],
                (1, 0, 2),
                false,
            ),
            (
                "an unexamined topic is not scored",
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
                .map(|(topic, _)| ExamQuestion {
                    topic: *topic,
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
            let topics: Vec<bool> = result.topics.iter().map(|s| s.passed).collect();
            assert_eq!(passes.to_vec(), topics, "{label}: topic passes");
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
            topics: Vec::new(),
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
    fn gaps_by_topic_keeps_each_failed_topics_own_misses_in_topic_order() {
        let grade = |verdict, missed: &[&str]| AnswerGrade {
            verdict,
            feedback: String::new(),
            missed: missed.iter().map(|point| point.to_string()).collect(),
        };
        let question = |topic| ExamQuestion {
            topic,
            ..Default::default()
        };
        let questions = [
            question(2),
            question(0),
            question(0),
            question(1),
            question(2),
        ];
        let result = ExamResult::scored(
            &questions,
            vec![
                grade(Verdict::Fail, &["c1", "dup"]),
                grade(Verdict::Partial, &["passed topic miss"]),
                grade(Verdict::Pass, &[]),
                grade(Verdict::Pass, &[]),
                grade(Verdict::Partial, &["dup"]),
            ],
            0.5,
        );
        let topics: Vec<ExamTopic> = ["topic-a", "topic-b", "topic-c"]
            .iter()
            .map(|id| ExamTopic {
                topic_id: id.to_string(),
                ..Default::default()
            })
            .collect();
        assert_eq!(
            vec![(
                "topic-c".to_string(),
                vec!["c1".to_string(), "dup".to_string()]
            )],
            result.gaps_by_topic(&questions, &topics),
            "topic-a passes at 1 of 2, so its partial's miss is not remediated; topic-c's misses dedupe"
        );
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
    fn generate_questions_rejects_a_deck_without_sources() {
        let ws = workspace_with("## q\na\n");
        let err = generate_questions(
            &opened(&ws),
            &ExamConfig::default(),
            &ask_config(std::path::Path::new("unused")),
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("declares no sources"),
            "{err:#}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_exam_reads_only_frozen_sources_never_exam_entries_cite_roots_or_cited_text() {
        let _lock = exec_lock();
        let ws = Workspace::new();
        let root = ws.root();
        for folder in ["code", "lib"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        std::fs::write(
            root.join(crate::workspace::MANIFEST),
            "[cite]\nshared = \"lib\"\n",
        )
        .unwrap();
        std::fs::write(root.join("own-exam.txt"), "DECK_EXAM_MARKER\n").unwrap();
        std::fs::write(root.join("code/cited.rs"), "CITED_TEXT_MARKER\n").unwrap();
        std::fs::write(root.join("code/uncited.rs"), "CITE_ROOT_MARKER\n").unwrap();
        std::fs::write(root.join("lib/shared.rs"), "MANIFEST_CITED_MARKER\n").unwrap();
        std::fs::write(
            ws.deck(),
            format!(
                "---\nformat-version: 1\nid: {DECK}\nexam: own-exam.txt\n\
                 cite:\n  - code: code\n---\n\
                 ## q1\na\n<!-- at: code:cited.rs:1 -->\n\n\
                 ## q2\nb\n<!-- at: shared:shared.rs:1 -->\n"
            ),
        )
        .unwrap();
        crate::assets::freeze_member(&ws.deck()).unwrap();
        crate::stamp::stamp_deck(&ws.deck()).unwrap();
        source(&ws, "frozen.txt", "FROZEN_SOURCE_MARKER\n", &[]);
        let deck = Deck::load(ws.deck()).unwrap();
        assert!(
            deck.cards
                .iter()
                .all(|card| card.citations.iter().all(|c| c.asset.is_some())),
            "setup: every citation is frozen"
        );

        let dir = tempfile::tempdir().unwrap();
        let args = dir.path().join("args.txt");
        let cli = crate::testutil::fake_cli(
            dir.path(),
            &format!(
                "input=$(cat); printf '%s\\n' \"$input\" >> {prompt}; printf '%s\\n' \"$@\" >> {args}\n\
                 case \"$input\" in\n\
                 *'\"questions\"'*) printf '%s' '{ONE_QUESTION}' ;;\n\
                 *) printf '%s' '{IDEAS}' ;;\n\
                 esac",
                prompt = dir.path().join("prompt.txt").display(),
                args = args.display(),
            ),
        );
        let mut ask_cfg = ask_config(&cli);
        ask_cfg.source_access = true;
        generate_questions(&deck, &ExamConfig::default(), &ask_cfg).unwrap();
        let seen = format!(
            "{}\n{}",
            std::fs::read_to_string(dir.path().join("prompt.txt")).unwrap(),
            std::fs::read_to_string(&args).unwrap()
        );

        let code_root = root.join("code").canonicalize().unwrap();
        let lib_root = root.join("lib").canonicalize().unwrap();
        let rows: [(&str, &str, bool); 7] = [
            ("the deck's frozen source", "FROZEN_SOURCE_MARKER", true),
            ("the deck's `exam:` entry", "DECK_EXAM_MARKER", false),
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
                "{label}: `{needle}` in the exam's prompts and arguments:\n{seen}"
            );
        }
    }

    #[test]
    fn a_deck_declares_sources_only_through_a_manifest_with_a_row() {
        let rows: [(&str, &str, bool, bool); 4] = [
            ("no sources folder", "", false, false),
            ("an `exam:` entry alone", "exam: e.txt\n", false, false),
            ("a manifest with a row", "", true, true),
            (
                "a trace deck with sources",
                "trace: how it works\n",
                true,
                false,
            ),
        ];
        for (label, frontmatter, with_source, expected) in rows {
            let ws = Workspace::new();
            std::fs::write(ws.root().join("e.txt"), "e\n").unwrap();
            std::fs::write(
                ws.deck(),
                format!("---\nid: \"{DECK}\"\n{frontmatter}---\n## q\na\n"),
            )
            .unwrap();
            if with_source {
                source(&ws, "s.txt", "s\n", &[]);
            }
            let deck = Deck::load(ws.deck()).unwrap();
            assert_eq!(expected, deck.declares_sources(), "{label}");
        }
    }

    #[test]
    fn workspace_asset_boundary_failures_surface_through_the_sitting() {
        let ws = Workspace::new();
        std::fs::write(
            ws.deck(),
            format!("---\nformat-version: 1\nid: {DECK}\ncite:\n  - src: https://example.org/source\n---\n## q\na\n<!-- at: src fingerprint: xxh64-0000000000000007 asset: missing.txt -->\n<!-- id: card-4jkya9q3m8z0tw5v9y2b4n6d8f -->\n"),
        )
        .unwrap();
        source(&ws, "s.txt", "s\n", &[]);
        let deck = Deck::load(ws.deck()).unwrap();

        let boundary = source_boundary_error(&deck).unwrap();
        assert!(boundary.contains("missing.txt"), "{boundary}");
        let sitting = Sitting::start(
            &deck,
            SittingStart::default(),
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
        sitting.accept_questions(Generated {
            questions: vec![
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
            ],
            ideas: crate::exam_ideas::IdeasDocument::default(),
        });

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

    #[test]
    fn an_answer_locks_once_the_learner_moves_on_and_a_resumed_sitting_keeps_it() {
        let mut sitting = test_sitting(Strictness::Balanced);
        sitting.accept_questions(Generated {
            questions: vec![ExamQuestion::default(), ExamQuestion::default()],
            ideas: crate::exam_ideas::IdeasDocument::default(),
        });
        sitting.set_answer("draft".to_string());
        sitting.set_answer("first".to_string());
        assert_eq!(
            "first",
            sitting.answers()[0],
            "step 1: editable before moving on"
        );
        sitting.next();
        sitting.goto(0);
        sitting.set_answer("changed".to_string());
        assert_eq!(
            "first",
            sitting.answers()[0],
            "step 2: locked after moving on"
        );
        assert!(
            sitting.is_locked(0) && !sitting.is_locked(1),
            "step 2: locks"
        );

        sitting.goto(1);
        sitting.next();
        assert!(!sitting.is_locked(1), "step 3: an empty answer never locks");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "# A\n\n## q?\na\n").unwrap();
        let deck = crate::deck::Deck::load(&path).unwrap();
        let resumed = Sitting::from_prepared(
            &deck,
            exam_topics(&deck, None),
            vec![ExamQuestion::default(); 3],
            vec!["one".to_string(), String::new(), String::new()],
            crate::exam_ideas::IdeasDocument::default(),
            SittingStart::default(),
            Strictness::Balanced,
            ExamConfig::default(),
            AskConfig::default(),
        );
        assert_eq!(
            (&Phase::Answering, 1, false),
            (resumed.phase(), resumed.current_index(), resumed.thinking()),
            "step 4: resumed at the first unanswered question, nothing pending"
        );
        assert!(
            resumed.is_locked(0) && !resumed.is_locked(1),
            "step 4: locks kept"
        );
    }

    #[test]
    fn opening_a_prepared_exam_does_not_start_question_generation() {
        let _lock = exec_lock();
        let ws = Workspace::new();
        let deck = sourced_deck(&ws);
        let before = GENERATION_SPAWNS.load(std::sync::atomic::Ordering::SeqCst);

        let sitting = Sitting::from_prepared(
            &deck,
            exam_topics(&deck, None),
            vec![ExamQuestion::default()],
            vec![String::new()],
            crate::exam_ideas::IdeasDocument::default(),
            SittingStart::default(),
            Strictness::Balanced,
            ExamConfig::default(),
            AskConfig::default(),
        );

        assert_eq!(&Phase::Answering, sitting.phase());
        assert_eq!(
            before,
            GENERATION_SPAWNS.load(std::sync::atomic::Ordering::SeqCst),
            "opening questions written ahead must not launch a fresh generation job"
        );
    }

    #[test]
    fn preparing_writes_the_exam_and_its_ideas_into_the_decks_store() {
        let _lock = exec_lock();
        let ws = workspace_with("# Moves\n<!-- id: topic-moves -->\n\n## Why move?\nownership\n");
        source(
            &ws,
            "src.md",
            "Moves transfer ownership.\n",
            &["topic-moves"],
        );
        let deck = opened(&ws);
        let cli_dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(
            cli_dir.path(),
            &[
                "{\"ideas\":[\"m\",\"n\"]}",
                "{\"questions\":[{\"shape\":\"instance\",\"idea\":2,\
                 \"prompt\":\"Q1\",\"points\":[\"p\"]},{\"shape\":\"fix\",\
                 \"idea\":1,\"prompt\":\"Q2\",\"points\":[\"p\"]}]}",
            ],
        );
        let exam_cfg = ExamConfig {
            model: Some("exam-model".to_string()),
            effort: Some("max".to_string()),
            ..ExamConfig::default()
        };
        let (rx, _job) = spawn_prepare(&deck, examined_topics(&deck), exam_cfg, ask_config(&cli));
        rx.recv().unwrap().unwrap();
        let (ideas_path, prepared_path) = exam_files(&deck).unwrap();
        let prepared = Prepared::load(&prepared_path).unwrap().unwrap();
        let ideas = IdeasDocument::load(&ideas_path).unwrap().unwrap();
        assert_eq!(
            vec![("topic-moves", 1, "n"), ("topic-moves", 0, "m")],
            prepared
                .questions
                .iter()
                .map(|q| (q.topic.as_str(), q.idea, q.mental_model.as_str()))
                .collect::<Vec<_>>(),
            "step 1: questions keyed by topic id and idea"
        );
        assert_eq!(
            vec![String::new(); 2],
            prepared.answers,
            "step 1: no answers yet"
        );
        let fingerprint = deck.declared().fingerprints()["topic-moves"].clone();
        assert_eq!(
            Some(&PreparedTopic {
                fingerprint: fingerprint.clone(),
                model: Some("exam-model".to_string()),
                effort: Some("max".to_string()),
            }),
            prepared.topics.get("topic-moves"),
            "step 1: the topic's fingerprint, model and effort"
        );
        assert_eq!(
            (fingerprint.as_str(), 2),
            (
                ideas.topics["topic-moves"].fingerprint.as_str(),
                ideas.topics["topic-moves"].ideas.len()
            ),
            "step 1: ideas kept under the same fingerprint"
        );

        source(&ws, "more.md", "Borrows lend.\n", &["topic-moves"]);
        let changed = opened(&ws);
        assert!(
            !prepared.is_current(&changed),
            "step 2: a new source makes the topic's questions stale"
        );
        let (topics, questions, answers) = prepared.against(&changed);
        assert!(
            topics.is_empty() && questions.is_empty() && answers.is_empty(),
            "step 2: a stale topic's questions are dropped"
        );
        let (topics, questions, _) = prepared.against(&deck);
        assert_eq!(
            (1, vec![0, 0]),
            (
                topics.len(),
                questions.iter().map(|q| q.topic).collect::<Vec<_>>()
            ),
            "step 3: unchanged sources keep them, re-indexed"
        );
    }

    #[test]
    fn failed_idea_persistence_does_not_leave_a_prepared_exam() {
        let _lock = exec_lock();
        let ws = workspace_with("# Moves\n<!-- id: topic-moves -->\n\n## Why move?\nownership\n");
        source(
            &ws,
            "src.md",
            "Moves transfer ownership.\n",
            &["topic-moves"],
        );
        let deck = opened(&ws);
        let (ideas_path, prepared_path) = exam_files(&deck).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(
            dir.path(),
            &format!(
                "input=$(cat)\n\
                 case \"$input\" in\n\
                 *'\"questions\"'*) mkdir -p {}; printf '%s' '{ONE_QUESTION}' ;;\n\
                 *) printf '%s' '{IDEAS}' ;;\n\
                 esac",
                ideas_path.display()
            ),
        );

        let (rx, _job) = spawn_prepare(
            &deck,
            examined_topics(&deck),
            ExamConfig::default(),
            ask_config(&cli),
        );

        let error = rx.recv().unwrap().unwrap_err();
        assert!(
            error.contains(&format!("writing {}", ideas_path.display())),
            "the idea save must be the failed step: {error}"
        );
        assert!(
            !prepared_path.exists(),
            "a failed preparation must not publish questions without their idea document"
        );
    }

    #[test]
    fn grading_records_each_asked_idea_under_its_topic_id() {
        let mut sitting = test_sitting(Strictness::Balanced);
        sitting.topics = vec![one_topic()];
        let mut ideas = IdeasDocument::default();
        ideas.for_topic("topic-moves", "f").ideas = ["a", "b", "c"]
            .into_iter()
            .map(|text| Idea {
                text: text.to_string(),
                ..Idea::default()
            })
            .collect();
        sitting.ideas = Some(ideas);
        sitting.questions = vec![
            ExamQuestion {
                idea: 2,
                ..Default::default()
            },
            ExamQuestion {
                idea: 0,
                ..Default::default()
            },
        ];
        let grade = |verdict| AnswerGrade {
            verdict,
            feedback: String::new(),
            missed: Vec::new(),
        };
        sitting.record_ideas(&ExamResult {
            grades: vec![grade(Verdict::Pass), grade(Verdict::Partial)],
            topics: Vec::new(),
        });
        let marked: Vec<_> = sitting.ideas().unwrap().topics["topic-moves"]
            .ideas
            .iter()
            .map(|i| (i.last_tested_ms.is_some(), i.last_test_passed))
            .collect();
        assert_eq!(
            vec![(true, Some(false)), (false, None), (true, Some(true))],
            marked,
            "a partial answer counts as not passed"
        );
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
            start: SittingStart::default(),
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
            remediated_topics: HashSet::new(),
            topics: Vec::new(),
            deck_topics: Vec::new(),
            unlocked: false,
            ideas: None,
            locked: vec![false],
            job: ask::AskJob::default(),
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

        let exam = ExamConfig {
            effort: Some("max".to_string()),
            ..exam
        };
        assert_eq!(
            Some("max".to_string()),
            run_config(&exam, &ask).effort,
            "the exam's own effort wins over [ask]"
        );
    }

    #[test]
    fn disconnected_exam_workers_have_a_specific_error() {
        assert_eq!("the exam helper exited unexpectedly", thread_gone());
    }

    #[test]
    fn generation_lists_ideas_then_questions_and_retries_each_call_once() {
        let _lock = exec_lock();
        let ideas = "{\"ideas\":[\"m\",\"n\"]}".to_string();
        let no_ideas = "{\"ideas\":[]}".to_string();
        let q = |idea: usize, prompt: &str, points: &str| {
            format!(
                "{{\"shape\":\"fix\",\"idea\":{idea},\
                 \"prompt\":\"{prompt}\",\"points\":[{points}]}}"
            )
        };
        let questions = |items: &[String]| format!("{{\"questions\":[{}]}}", items.join(","));
        let good = questions(&[q(1, "Q1", "\"p\""), q(2, "Q2", "\"p\"")]);
        let only_m = questions(&[q(1, "Q", "\"p\"")]);
        let rows: Vec<(&str, Vec<String>, usize, Option<&str>)> = vec![
            ("good first time", vec![ideas.clone(), good.clone()], 2, None),
            (
                "ideas bad then good",
                vec![no_ideas.clone(), ideas.clone(), good.clone()],
                3,
                None,
            ),
            (
                "questions bad then good",
                vec![ideas.clone(), only_m.clone(), good.clone()],
                3,
                None,
            ),
            (
                "no idea listed, twice",
                vec![no_ideas.clone(), "{\"ideas\":[\" \"]}".to_string()],
                2,
                Some("the listed ideas were rejected twice: it lists no idea"),
            ),
            (
                "the ideas under another key, twice",
                vec![
                    "{\"topics\":[{\"topic\":1,\"models\":[\"m\"]}]}".to_string(),
                    "{\"ideas\":[\"m\"],\"topics\":[]}".to_string(),
                ],
                2,
                Some("the listed ideas were rejected twice: it was not the JSON asked for"),
            ),
            (
                "a question naming a topic, twice",
                vec![
                    ideas.clone(),
                    "{\"questions\":[{\"topic\":1,\"shape\":\"fix\",\"idea\":1,\"prompt\":\"Q\",\"points\":[\"p\"]}]}".to_string(),
                    "{\"questions\":[{\"topic\":1,\"shape\":\"fix\",\"idea\":1,\"prompt\":\"Q\",\"points\":[\"p\"]}]}".to_string(),
                ],
                3,
                Some("the generated questions were rejected twice: it was not the JSON asked for"),
            ),
            (
                "an unlisted idea, twice",
                vec![
                    ideas.clone(),
                    questions(&[q(9, "Q", "\"p\"")]),
                    questions(&[q(0, "Q", "\"p\"")]),
                ],
                3,
                Some("names idea 0, but the topic lists 2"),
            ),
            (
                "the same idea twice, twice",
                vec![
                    ideas.clone(),
                    questions(&[q(1, "Q1", "\"p\""), q(1, "Q2", "\"p\"")]),
                    questions(&[q(1, "Q1", "\"p\""), q(1, "Q2", "\"p\"")]),
                ],
                3,
                Some("idea 1 is asked more than once"),
            ),
            (
                "an uncovered idea, twice",
                vec![ideas.clone(), only_m.clone(), only_m.clone()],
                3,
                Some("the idea `n` is listed but no question asks about it"),
            ),
            (
                "an empty prompt, then no key points",
                vec![
                    ideas.clone(),
                    questions(&[q(1, " ", "\"p\""), q(2, "Q", "\"p\"")]),
                    questions(&[q(1, "Q", ""), q(2, "Q", "\"p\"")]),
                ],
                3,
                Some("empty prompt or no key points"),
            ),
            (
                "a blank key point, twice",
                vec![
                    ideas.clone(),
                    questions(&[q(1, "Q", "\" \""), q(2, "Q", "\"p\"")]),
                    questions(&[q(1, "Q", "\"\""), q(2, "Q", "\"p\"")]),
                ],
                3,
                Some("empty prompt or no key points"),
            ),
        ];
        let ws = workspace_with("## Why move?\nownership\n");
        source(&ws, "moves.txt", "Moves transfer ownership.\n", &[]);
        let deck = opened(&ws);
        for (label, replies, calls, failure) in rows {
            let dir = tempfile::tempdir().unwrap();
            let replies: Vec<&str> = replies.iter().map(String::as_str).collect();
            let cli = crate::testutil::fake_replies(dir.path(), &replies);
            let result = generate_questions(&deck, &ExamConfig::default(), &ask_config(&cli));
            assert_eq!(
                calls,
                crate::testutil::fake_reply_calls(dir.path()),
                "{label}: calls"
            );
            match (failure, result) {
                (None, Ok(questions)) => {
                    let tested: Vec<&str> =
                        questions.iter().map(|q| q.mental_model.as_str()).collect();
                    assert_eq!(
                        vec!["m", "n"],
                        tested,
                        "{label}: every listed idea is tested"
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
    fn the_question_call_gets_the_listed_ideas_and_a_retry_names_the_rejection() {
        let _lock = exec_lock();
        let ws = workspace_with("## Why move?\nownership\n");
        source(&ws, "moves.txt", "Moves transfer ownership.\n", &[]);
        let dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(
            dir.path(),
            &[
                "{\"ideas\":[]}",
                "{\"ideas\":[\"moves transfer ownership\"]}",
                ONE_QUESTION,
            ],
        );
        generate_questions(&opened(&ws), &ExamConfig::default(), &ask_config(&cli)).unwrap();
        let read =
            |n: usize| std::fs::read_to_string(dir.path().join(format!("prompt-{n}"))).unwrap();
        assert!(
            read(1).contains("List the topic's ideas"),
            "call 1 lists ideas: {}",
            read(1)
        );
        assert!(
            read(2).contains("Your previous answer was rejected: it lists no idea"),
            "call 2 retries the listing with the reason: {}",
            read(2)
        );
        assert!(
            read(3).contains("1. moves transfer ownership"),
            "call 3 writes questions for the listed ideas: {}",
            read(3)
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
        let ws = workspace_with("## Why move?\nownership\n");
        source(&ws, "moves.txt", "Moves transfer ownership.\n", &[]);
        let dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(dir.path(), &[IDEAS, ONE_QUESTION]);
        let qs =
            generate_questions(&opened(&ws), &ExamConfig::default(), &ask_config(&cli)).unwrap();
        assert_eq!(1, qs.len());
        assert_eq!("Why move?", qs[0].prompt);
        assert_eq!(vec!["avoids double free"], qs[0].points);
        assert_eq!(
            (0, Shape::Instance, "m"),
            (qs[0].topic, qs[0].shape, qs[0].mental_model.as_str()),
            "the topic is an index; the shape and the idea's text are kept"
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
             *'\"questions\"'*) printf '%s' '{{\"questions\":[{{\"shape\":\"instance\",\"idea\":1,\"prompt\":\"Q1\",\"points\":[\"p1\"]}},{{\"shape\":\"boundary\",\"idea\":2,\"prompt\":\"Q2\",\"points\":[\"p2\"]}}]}}' ;;\n\
             *'\"ideas\"'*) printf '%s' '{{\"ideas\":[\"m\",\"n\"]}}' ;;\n\
             *) printf '## Why does X?\\npoint one\\n' ;;\n\
             esac"
        );
        fake_cli(dir, &body)
    }

    fn sidecar_cards(deck_path: &std::path::Path, deck_id: &str) -> Vec<crate::card::Card> {
        let text =
            std::fs::read_to_string(crate::personal::sidecar_path(deck_path)).unwrap_or_default();
        parser::parse_sidecar(deck_id, &text)
            .map(|parsed| parsed.cards.into_iter().map(|parsed| parsed.card).collect())
            .unwrap_or_default()
    }

    fn sourced_deck(ws: &Workspace) -> Deck {
        std::fs::write(
            ws.deck(),
            format!("---\nformat-version: 1\nid: \"{DECK}\"\n---\n## c\na\n<!-- id: card-qc -->\n"),
        )
        .unwrap();
        source(ws, "s.txt", "c stands for a concept.\n", &[]);
        Deck::load(ws.deck()).unwrap()
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
        let ws = Workspace::new();
        let deck = sourced_deck(&ws);
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let mut s = Sitting::start(
            &deck,
            sitting_start(&deck, &store, 0, None),
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
        assert!(!store.deck_passed(DECK));
        assert_eq!(
            Some(false),
            store.topic_result(DECK, DECK).map(|r| r.pass),
            "a failed topic is recorded by its topic id"
        );
        assert!(!s.unlocked());

        assert!(s.can_remediate());
        let snapshot = std::fs::read(ws.deck()).unwrap();
        s.remediate();
        assert_eq!(&Phase::Remediating, s.phase());
        drain(&mut s, &mut store);
        assert_eq!(&Phase::Remediated, s.phase());
        assert_eq!(snapshot, std::fs::read(ws.deck()).unwrap());
        let personal = sidecar_cards(&ws.deck(), DECK);
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
        let ws = Workspace::new();
        let deck = sourced_deck(&ws);
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let mut s = Sitting::start(
            &deck,
            sitting_start(&deck, &store, 0, None),
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

        let personal = sidecar_cards(&ws.deck(), DECK);
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
        let ws = Workspace::new();
        let deck = sourced_deck(&ws);
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let mut s = Sitting::start(
            &deck,
            sitting_start(&deck, &store, 0, None),
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
        assert!(store.deck_passed(DECK), "every topic passed: passed_at_ms");
        assert!(
            store.deck_mastered(DECK),
            "a full sitting at 100%: mastered_at_ms"
        );
        assert!(s.unlocked(), "this sitting set passed_at_ms");
        assert_eq!(
            Some(true),
            store.topic_result(DECK, DECK).map(|r| r.pass),
            "the topic's verdict is recorded by its topic id"
        );
        assert_eq!(
            Some((2, 0, 2)),
            store
                .full_exam_result(DECK)
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
        let ws = Workspace::new();
        let deck = sourced_deck(&ws);
        let deck_path = ws.deck();
        let before = std::fs::read(&deck_path).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let mut s = Sitting::start(
            &deck,
            sitting_start(&deck, &store, 0, None),
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
            sidecar.contains(&format!("for: {DECK}")),
            "the sidecar names its deck: {sidecar}"
        );
        let cards = sidecar_cards(&deck_path, DECK);
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
        let ws = Workspace::new();
        let deck = sourced_deck(&ws);
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let mut s = Sitting::start(
            &deck,
            sitting_start(&deck, &store, 0, None),
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
        assert!(!sidecar_cards(&ws.deck(), DECK).is_empty());

        let cli_dir2 = tempfile::tempdir().unwrap();
        let cli2 = branching_cli(
            cli_dir2.path(),
            "{\"grades\":[{\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]},\
             {\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]}]}",
        );
        let mut s2 = Sitting::start(
            &deck,
            sitting_start(&deck, &store, 0, None),
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
        assert!(store.deck_passed(DECK));

        for card in sidecar_cards(&ws.deck(), DECK) {
            let id = card.id().expect("a personal card carries its id");
            assert!(
                !is_retired_id(&id, &store, Some(crate::session::DEFAULT_RETIRE_AFTER_DAYS)),
                "a passing re-sit must not retire remediation cards"
            );
        }
    }

    fn two_failed_topics(ws: &Workspace, cli: &std::path::Path) -> Sitting {
        let deck = opened(ws);
        let topics = examined_topics(&deck);
        let index = |id: &str| {
            topics
                .iter()
                .position(|topic| topic.topic_id == id)
                .unwrap_or_else(|| panic!("{id} is examined"))
        };
        let questions = vec![
            ExamQuestion {
                prompt: "qa".to_string(),
                topic: index("topic-alpha"),
                ..Default::default()
            },
            ExamQuestion {
                prompt: "qb".to_string(),
                topic: index("topic-beta"),
                ..Default::default()
            },
        ];
        let fail = |missed: &str| AnswerGrade {
            verdict: Verdict::Fail,
            feedback: String::new(),
            missed: vec![missed.to_string()],
        };
        let result = ExamResult::scored(&questions, vec![fail("alpha"), fail("beta")], 1.0);
        let mut s = test_sitting(Strictness::Balanced);
        s.deck_path = Some(ws.deck());
        s.deck_token = DECK.to_string();
        s.ask_cfg = ask_config(cli);
        s.phase = Phase::Results;
        s.questions = questions;
        s.answers = vec![String::new(); 2];
        s.locked = vec![false; 2];
        s.result = Some(result);
        s.topics = topics;
        s
    }

    #[test]
    fn a_remediation_retry_after_a_partial_failure_writes_no_duplicate() {
        let _lock = exec_lock();
        let ws = two_sourced_topics();
        let cli_dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(
            cli_dir.path(),
            &[
                "## Alpha gap?\nA\n",
                "## q\n",
                "## Beta gap?\nB\n",
                "## Extra?\nE\n",
            ],
        );
        let mut s = two_failed_topics(&ws, &cli);
        let mut store = Store::open(cli_dir.path().join("p.json")).unwrap();

        s.remediate();
        drain(&mut s, &mut store);
        assert!(s.error().is_some(), "first try: beta's block is malformed");
        assert!(s.can_remediate(), "first try: beta is still owed");

        s.remediate();
        drain(&mut s, &mut store);
        assert_eq!(None, s.error(), "retry");

        let reloaded = Deck::load(ws.deck()).unwrap();
        let topic_ids = reloaded.topic_ids();
        let bound = crate::assemble::personal_cards(&reloaded);
        assert_eq!(
            vec![
                ("Alpha gap?".to_string(), Some("topic-alpha".to_string())),
                ("Beta gap?".to_string(), Some("topic-beta".to_string())),
            ],
            bound
                .cards
                .iter()
                .map(|card| (card.front.clone(), topic_ids[card.topic].clone()))
                .collect::<Vec<_>>(),
            "retry: alpha is not remediated again"
        );
    }

    #[test]
    fn remediation_runs_one_call_per_failed_topic_and_binds_each_card_to_it() {
        let _lock = exec_lock();
        let ws = two_sourced_topics();
        let deck = opened(&ws);
        let cli_dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(
            cli_dir.path(),
            &["## Alpha gap?\nA\n", "## Beta gap?\nB\n"],
        );
        let topics = examined_topics(&deck);
        let index = |id: &str| {
            topics
                .iter()
                .position(|topic| topic.topic_id == id)
                .unwrap_or_else(|| panic!("{id} is examined"))
        };
        let questions = vec![
            ExamQuestion {
                prompt: "qa".to_string(),
                topic: index("topic-alpha"),
                ..Default::default()
            },
            ExamQuestion {
                prompt: "qb".to_string(),
                topic: index("topic-beta"),
                ..Default::default()
            },
        ];
        let fail = |missed: &str| AnswerGrade {
            verdict: Verdict::Fail,
            feedback: String::new(),
            missed: vec![missed.to_string()],
        };
        let result = ExamResult::scored(&questions, vec![fail("alpha"), fail("beta")], 1.0);
        let mut s = test_sitting(Strictness::Balanced);
        s.deck_path = Some(ws.deck());
        s.deck_token = DECK.to_string();
        s.ask_cfg = ask_config(&cli);
        s.phase = Phase::Results;
        s.questions = questions;
        s.answers = vec![String::new(); 2];
        s.locked = vec![false; 2];
        s.result = Some(result);
        s.topics = topics;
        let mut store = Store::open(cli_dir.path().join("p.json")).unwrap();

        s.remediate();
        drain(&mut s, &mut store);

        assert_eq!(
            2,
            crate::testutil::fake_reply_calls(cli_dir.path()),
            "one call per failed topic"
        );
        assert_eq!(Some(2), s.remediated_count(), "error: {:?}", s.error());
        let reloaded = Deck::load(ws.deck()).unwrap();
        let topic_ids = reloaded.topic_ids();
        let bound = crate::assemble::personal_cards(&reloaded);
        assert_eq!(
            vec![
                ("Alpha gap?".to_string(), Some("topic-alpha".to_string())),
                ("Beta gap?".to_string(), Some("topic-beta".to_string())),
            ],
            bound
                .cards
                .iter()
                .map(|card| (card.front.clone(), topic_ids[card.topic].clone()))
                .collect::<Vec<_>>(),
            "each card binds to the topic it remediates: {:?}",
            bound.warnings
        );
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
        let ws = Workspace::new();
        let deck = sourced_deck(&ws);
        let mut store = Store::open(dir.path().join("p.json")).unwrap();

        let mut s = Sitting::start(
            &deck,
            sitting_start(&deck, &store, 0, None),
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
        let Some(Effect::RemediationCards(by_topic)) = effect else {
            panic!("expected Some(Effect::RemediationCards(_)), got {effect:?}");
        };
        let [topic] = by_topic.as_slice() else {
            panic!("one failed topic gives one entry: {by_topic:?}");
        };
        let synth = parser::parse_str("d.md", &topic.cards).unwrap();
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
        let ws = Workspace::new();
        let deck = sourced_deck(&ws);

        let mut s = Sitting::start(
            &deck,
            SittingStart::default(),
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
        let ws = Workspace::new();
        let deck = sourced_deck(&ws);

        let mut s = Sitting::start(
            &deck,
            SittingStart::default(),
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

    use crate::sources::{
        self,
        testkit::{DECK, Workspace},
    };

    const TWO_TOPICS: &str = "# Alpha\n\nAlpha prose.\n<!-- id: topic-alpha -->\n\n## a1?\none\n\n\
                              # Beta\n<!-- id: topic-beta -->\n\n## b1?\ntwo\n";

    fn workspace_with(body: &str) -> Workspace {
        let ws = Workspace::new();
        std::fs::write(
            ws.deck(),
            format!("---\nid: \"{DECK}\"\ntitle: Ownership\n---\n{body}"),
        )
        .unwrap();
        ws
    }

    fn source(ws: &Workspace, name: &str, text: &str, topics: &[&str]) {
        ws.write_live(name, text.as_bytes());
        ws.add(&[name], topics).unwrap();
    }

    fn two_sourced_topics() -> Workspace {
        let ws = workspace_with(TWO_TOPICS);
        source(&ws, "alpha.txt", "ALPHA_SOURCE\n", &["topic-alpha"]);
        source(&ws, "beta.txt", "BETA_SOURCE\n", &["topic-beta"]);
        ws
    }

    fn opened(ws: &Workspace) -> Deck {
        open_for_exam(&ws.deck()).unwrap()
    }

    fn prompts(dir: &std::path::Path) -> Vec<String> {
        (1..=crate::testutil::fake_reply_calls(dir))
            .map(|n| std::fs::read_to_string(dir.join(format!("prompt-{n}"))).unwrap())
            .collect()
    }

    const IDEAS: &str = "{\"ideas\":[\"m\"]}";
    const ONE_QUESTION: &str = "{\"questions\":[{\"shape\":\"instance\",\"idea\":1,\
                                \"prompt\":\"Why move?\",\"points\":[\"avoids double free\"]}]}";

    fn prepare(deck: &Deck, replies: &[&str]) -> (tempfile::TempDir, Vec<String>) {
        let cli_dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(cli_dir.path(), replies);
        let (rx, _job) = spawn_prepare(
            deck,
            examined_topics(deck),
            ExamConfig::default(),
            ask_config(&cli),
        );
        rx.recv().unwrap().unwrap();
        let seen = prompts(cli_dir.path());
        (cli_dir, seen)
    }

    fn one_topic() -> ExamTopic {
        ExamTopic {
            topic_id: "topic-moves".to_string(),
            label: "Moves".to_string(),
            context: vec!["Moves".to_string()],
            cards: vec!["Q: a\nA: b".to_string()],
            sourced: true,
            ..Default::default()
        }
    }

    fn one_source() -> Vec<SourceText> {
        vec![SourceText {
            display: "sdk:src/lib.rs".to_string(),
            text: "fn main() {}\n".to_string(),
        }]
    }

    #[test]
    fn adding_a_source_to_one_topic_regenerates_only_that_topics_ideas() {
        let _lock = exec_lock();
        let ws = two_sourced_topics();
        let (_first, seen) = prepare(&opened(&ws), &[IDEAS, ONE_QUESTION, IDEAS, ONE_QUESTION]);
        let listing_calls = |seen: &[String], label: &str| {
            seen.iter()
                .filter(|p| p.contains("\"ideas\"") && p.contains(&format!("Topic: {label}\n")))
                .count()
        };
        let question_calls = |seen: &[String], label: &str| {
            seen.iter()
                .filter(|p| p.contains("\"questions\"") && p.contains(&format!("Topic: {label}\n")))
                .count()
        };
        assert_eq!(
            (1, 1, 1, 1),
            (
                listing_calls(&seen, "Alpha"),
                question_calls(&seen, "Alpha"),
                listing_calls(&seen, "Beta"),
                question_calls(&seen, "Beta")
            ),
            "step 1: one listing and one question call per topic"
        );

        source(&ws, "beta-more.txt", "BETA_MORE\n", &["topic-beta"]);
        let deck = opened(&ws);
        assert!(
            !has_current_prepared(&deck),
            "step 2: Beta's questions are stale"
        );
        let (_second, seen) = prepare(&deck, &[IDEAS, ONE_QUESTION]);
        assert_eq!(
            (0, 0, 1, 1, 2),
            (
                listing_calls(&seen, "Alpha"),
                question_calls(&seen, "Alpha"),
                listing_calls(&seen, "Beta"),
                question_calls(&seen, "Beta"),
                seen.len()
            ),
            "step 2: only Beta is listed and asked again"
        );
        assert!(has_current_prepared(&deck), "step 2: current again");
        let (_, prepared_path) = exam_files(&deck).unwrap();
        let prepared = Prepared::load(&prepared_path).unwrap().unwrap();
        assert_eq!(
            vec!["topic-alpha", "topic-beta"],
            prepared
                .questions
                .iter()
                .map(|q| q.topic.as_str())
                .collect::<Vec<_>>(),
            "step 2: Alpha's question is kept beside Beta's new one"
        );
    }

    fn prepare_over(
        deck: &Deck,
        topics: Vec<ExamTopic>,
        replies: &[&str],
    ) -> (tempfile::TempDir, Result<(), String>) {
        let cli_dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(cli_dir.path(), replies);
        let (rx, _job) = spawn_prepare(deck, topics, ExamConfig::default(), ask_config(&cli));
        let outcome = rx.recv().unwrap();
        (cli_dir, outcome)
    }

    fn ideas_of(deck: &Deck) -> IdeasDocument {
        let (ideas_path, _) = exam_files(deck).unwrap();
        IdeasDocument::load(&ideas_path)
            .unwrap()
            .unwrap_or_default()
    }

    #[test]
    fn a_topic_sourced_after_generate_makes_the_prepared_exam_stale() {
        let _lock = exec_lock();
        let ws = workspace_with(TWO_TOPICS);
        source(&ws, "alpha.txt", "ALPHA_SOURCE\n", &["topic-alpha"]);
        prepare(&opened(&ws), &[IDEAS, ONE_QUESTION]);
        assert!(
            has_current_prepared(&opened(&ws)),
            "step 1: Alpha alone is prepared and current"
        );
        source(&ws, "beta.txt", "BETA_SOURCE\n", &["topic-beta"]);
        assert!(
            !has_current_prepared(&opened(&ws)),
            "step 2: a newly sourced Beta makes the prepared exam stale, so Generate is open"
        );

        let subset = two_sourced_topics();
        let deck = opened(&subset);
        let alpha_only: Vec<ExamTopic> = examined_topics(&deck)
            .into_iter()
            .filter(|topic| topic.topic_id == "topic-alpha")
            .collect();
        let (_cli, outcome) = prepare_over(&deck, alpha_only, &[IDEAS, ONE_QUESTION]);
        assert_eq!(Ok(()), outcome, "needs-work prepare over Alpha only");
        assert!(
            has_current_prepared(&opened(&subset)),
            "a prepared exam that asked only Alpha stays current while nothing changes"
        );
    }

    #[test]
    fn unbinding_a_topics_last_source_drops_its_ideas_on_the_next_prepare() {
        let _lock = exec_lock();
        let ws = workspace_with(TWO_TOPICS);
        source(&ws, "alpha.txt", "ALPHA_SOURCE\n", &["topic-alpha"]);
        source(
            &ws,
            "shared.txt",
            "SHARED\n",
            &["topic-alpha", "topic-beta"],
        );
        prepare(&opened(&ws), &[IDEAS, ONE_QUESTION, IDEAS, ONE_QUESTION]);
        assert!(
            ideas_of(&opened(&ws)).topics.contains_key("topic-beta"),
            "setup: Beta has ideas"
        );
        let shared = opened(&ws)
            .sources()
            .unwrap()
            .unwrap()
            .manifest
            .sources
            .iter()
            .find(|(_, row)| row.path.ends_with("shared.txt"))
            .map(|(id, _)| id.clone())
            .unwrap();
        crate::sources::commands::unbind(&ws.deck(), &shared, &["topic-beta".to_string()]).unwrap();
        let deck = opened(&ws);
        prepare(&deck, &[IDEAS, ONE_QUESTION]);
        assert_eq!(
            vec!["topic-alpha"],
            ideas_of(&deck)
                .topics
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            "the unbound topic's ideas are pruned"
        );
    }

    #[test]
    fn a_failing_topic_keeps_the_ideas_of_the_topics_before_it() {
        let _lock = exec_lock();
        let ws = two_sourced_topics();
        let deck = opened(&ws);
        let (_cli, outcome) = prepare_over(
            &deck,
            examined_topics(&deck),
            &[IDEAS, ONE_QUESTION, "{}", "{}"],
        );
        assert!(outcome.is_err(), "Beta's listing fails twice: {outcome:?}");
        assert_eq!(
            vec!["topic-alpha"],
            ideas_of(&deck)
                .topics
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            "Alpha's ideas were saved before Beta failed"
        );
    }

    #[test]
    fn resuming_a_prepared_exam_refuses_a_duplicated_topic_id_like_a_fresh_sitting() {
        let _lock = exec_lock();
        let ws = two_sourced_topics();
        prepare(&opened(&ws), &[IDEAS, ONE_QUESTION, IDEAS, ONE_QUESTION]);
        let text = std::fs::read_to_string(ws.deck()).unwrap();
        std::fs::write(
            ws.deck(),
            format!("{text}\n# Gamma\n<!-- id: topic-beta -->\n\n## g?\nthree\n"),
        )
        .unwrap();
        let deck = Deck::load(ws.deck()).unwrap();
        let (_, prepared_path) = exam_files(&deck).unwrap();
        let prepared = Prepared::load(&prepared_path).unwrap().unwrap();
        let (topics, questions, answers) = prepared.against(&deck);
        let sitting = Sitting::from_prepared(
            &deck,
            topics,
            questions,
            answers,
            IdeasDocument::default(),
            SittingStart::default(),
            Strictness::Balanced,
            ExamConfig::default(),
            AskConfig::default(),
        );
        assert_eq!(
            duplicated_topic_error(&deck).as_deref(),
            sitting.error(),
            "the resume refuses with the shared findings text"
        );
        assert!(sitting.error().is_some(), "setup: the id is duplicated");
    }

    #[test]
    fn a_binding_to_a_topic_the_deck_lacks_refuses_prepare_and_every_sitting_start() {
        let _lock = exec_lock();
        let ws = two_sourced_topics();
        prepare(&opened(&ws), &[IDEAS, ONE_QUESTION, IDEAS, ONE_QUESTION]);
        std::fs::write(
            ws.deck(),
            format!(
                "---\nid: \"{DECK}\"\ntitle: Ownership\n---\n\
                 # Alpha\n\nAlpha prose.\n<!-- id: topic-alpha -->\n\n## a1?\none\n"
            ),
        )
        .unwrap();
        let deck = opened(&ws);
        let expected = deck
            .sources()
            .unwrap()
            .unwrap()
            .validate_bindings(&deck)
            .unwrap_err()
            .to_string();
        assert!(expected.contains("topic-beta"), "setup: {expected}");
        assert!(
            !has_current_prepared(&deck),
            "a prepared question whose topic the deck lost is not current"
        );

        let (_cli, outcome) = prepare_over(&deck, examined_topics(&deck), &[IDEAS, ONE_QUESTION]);
        assert_eq!(Err(expected.clone()), outcome, "prepare");

        let cli_dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(cli_dir.path(), &[IDEAS, ONE_QUESTION]);
        let fresh = Sitting::start(
            &deck,
            SittingStart::default(),
            Strictness::Balanced,
            ExamConfig::default(),
            ask_config(&cli),
        );
        assert_eq!(Some(expected.as_str()), fresh.error(), "fresh sitting");
        assert_eq!(
            0,
            crate::testutil::fake_reply_calls(cli_dir.path()),
            "no model call"
        );

        let (_, prepared_path) = exam_files(&deck).unwrap();
        let prepared = Prepared::load(&prepared_path).unwrap().unwrap();
        let (topics, questions, answers) = prepared.against(&deck);
        let resumed = Sitting::from_prepared(
            &deck,
            topics,
            questions,
            answers,
            IdeasDocument::default(),
            SittingStart::default(),
            Strictness::Balanced,
            ExamConfig::default(),
            AskConfig::default(),
        );
        assert_eq!(Some(expected.as_str()), resumed.error(), "resumed sitting");
    }

    #[test]
    fn each_topic_prompt_holds_only_its_own_sources_sorted_by_display_path() {
        let _lock = exec_lock();
        let ws = two_sourced_topics();
        let mut n = 0;
        let (dl_text, sdk_text) = loop {
            let dl = format!("DL_SOURCE {n}\n");
            if sources::source_id(dl.as_bytes()) > sources::source_id(b"SDK_SOURCE\n") {
                break (dl, "SDK_SOURCE\n");
            }
            n += 1;
        };
        let other = ws.root().join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("dl.txt"), &dl_text).unwrap();
        sources::commands::add(sources::commands::AddRequest {
            deck: &ws.deck(),
            files: vec![other.join("dl.txt")],
            root: Some(("dl".to_string(), other.clone())),
            key: None,
            topics: vec!["topic-alpha".to_string()],
        })
        .unwrap();
        source(&ws, "sdk.txt", sdk_text, &["topic-alpha"]);
        let cli_dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(
            cli_dir.path(),
            &[IDEAS, ONE_QUESTION, IDEAS, ONE_QUESTION],
        );
        generate_questions(&opened(&ws), &ExamConfig::default(), &ask_config(&cli)).unwrap();
        let seen = prompts(cli_dir.path());
        let alpha = &seen[0];
        let beta = &seen[2];
        assert!(
            alpha.contains("ALPHA_SOURCE") && !alpha.contains("BETA_SOURCE"),
            "Alpha's prompt holds Alpha's bytes only: {alpha}"
        );
        assert!(
            beta.contains("BETA_SOURCE") && !beta.contains("ALPHA_SOURCE"),
            "Beta's prompt holds Beta's bytes only: {beta}"
        );
        let at = |needle: &str| alpha.find(needle).unwrap_or(usize::MAX);
        assert!(
            at("Source `dl:dl.txt`") < at("Source `sdk:alpha.txt`")
                && at("Source `sdk:alpha.txt`") < at("Source `sdk:sdk.txt`"),
            "sources enter by display path, not by id: {alpha}"
        );
    }

    #[test]
    fn a_renamed_heading_keeps_its_bindings_ideas_and_prepared_questions() {
        let _lock = exec_lock();
        let ws = two_sourced_topics();
        prepare(&opened(&ws), &[IDEAS, ONE_QUESTION, IDEAS, ONE_QUESTION]);
        let text = std::fs::read_to_string(ws.deck()).unwrap();
        std::fs::write(ws.deck(), text.replace("# Alpha", "# Alpha renamed")).unwrap();
        let renamed = opened(&ws);
        assert!(
            renamed.sources().unwrap().is_some(),
            "the bindings still name the topic id"
        );
        let (_, prepared_path) = exam_files(&renamed).unwrap();
        let prepared = Prepared::load(&prepared_path).unwrap().unwrap();
        let (topics, questions, _) = prepared.against(&renamed);
        assert_eq!(
            vec![("topic-alpha", "Alpha renamed"), ("topic-beta", "Beta")],
            topics
                .iter()
                .map(|t| (t.topic_id.as_str(), t.label.as_str()))
                .collect::<Vec<_>>(),
            "the prepared questions follow the topic id under its new heading"
        );
        assert_eq!(2, questions.len());
        assert!(has_current_prepared(&renamed));
        let (ideas_path, _) = exam_files(&renamed).unwrap();
        let ideas = IdeasDocument::load(&ideas_path).unwrap().unwrap();
        assert_eq!(
            vec!["m".to_string()],
            ideas.topics["topic-alpha"]
                .ideas
                .iter()
                .map(|i| i.text.clone())
                .collect::<Vec<_>>(),
            "the idea cache is keyed by the topic id"
        );
    }

    #[test]
    fn same_text_headings_are_examined_as_two_topics() {
        let ws = workspace_with(
            "# A\n<!-- id: topic-first -->\n\n## a1?\none\n\n# B\n<!-- id: topic-b -->\n\n## \
             b1?\ntwo\n\n# A\n<!-- id: topic-second -->\n\n## a2?\nthree\n",
        );
        let deck = opened(&ws);
        let topics = exam_topics(&deck, None);
        assert_eq!(
            vec![
                (DECK, "Ownership", 0),
                ("topic-first", "A", 1),
                ("topic-b", "B", 1),
                ("topic-second", "A", 1)
            ],
            topics
                .iter()
                .map(|t| (t.topic_id.as_str(), t.label.as_str(), t.cards.len()))
                .collect::<Vec<_>>(),
            "one entry per topic in deck order, the leading topic first, never merged"
        );
        assert!(topics[1].cards[0].contains("a1?") && topics[3].cards[0].contains("a2?"));
    }

    #[test]
    fn a_deck_copied_to_a_machine_without_the_live_files_builds_byte_identical_topic_prompts() {
        let _lock = exec_lock();
        let ws = two_sourced_topics();
        let run = |deck_path: &std::path::Path| {
            let cli_dir = tempfile::tempdir().unwrap();
            let cli = crate::testutil::fake_replies(
                cli_dir.path(),
                &[IDEAS, ONE_QUESTION, IDEAS, ONE_QUESTION],
            );
            let deck = open_for_exam(deck_path).unwrap();
            generate_questions(&deck, &ExamConfig::default(), &ask_config(&cli)).unwrap();
            prompts(cli_dir.path())
        };
        let here = run(&ws.deck());
        let there = tempfile::tempdir().unwrap();
        std::fs::write(there.path().join("alix.toml"), "").unwrap();
        for folder in ["decks", "sources"] {
            let from = ws.root().join(folder);
            for entry in walk(&from) {
                let to = there
                    .path()
                    .join(folder)
                    .join(entry.strip_prefix(&from).unwrap());
                std::fs::create_dir_all(to.parent().unwrap()).unwrap();
                std::fs::copy(&entry, &to).unwrap();
            }
        }
        let roots = there.path().join("sources").join(DECK).join(sources::ROOTS);
        std::fs::remove_file(roots).unwrap();
        std::fs::remove_dir_all(ws.live()).unwrap();
        let away = run(&there.path().join("decks").join("d.md"));
        assert_eq!(2 * 2, here.len(), "two calls per topic");
        assert_eq!(
            here, away,
            "the prompts are byte-identical without the live files"
        );
    }

    fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.extend(walk(&path));
            } else {
                files.push(path);
            }
        }
        files
    }

    #[test]
    fn an_unsourced_topic_is_never_sent_to_the_model_and_a_deck_with_no_sourced_topic_sits_no_exam()
    {
        let _lock = exec_lock();
        let ws = workspace_with(TWO_TOPICS);
        source(&ws, "alpha.txt", "ALPHA_SOURCE\n", &["topic-alpha"]);
        let deck = opened(&ws);
        let cli_dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(cli_dir.path(), &[IDEAS, ONE_QUESTION]);
        let mut sitting = Sitting::start(
            &deck,
            SittingStart::default(),
            Strictness::Balanced,
            ExamConfig::default(),
            ask_config(&cli),
        );
        assert_eq!(None, advance_until_idle(&mut sitting));
        assert_eq!(None, sitting.error(), "the sourced topic is examined");
        let seen = prompts(cli_dir.path());
        assert_eq!(2, seen.len(), "one topic, two calls");
        assert!(
            seen.iter()
                .all(|p| !p.contains("Beta") && !p.contains("b1?")),
            "the unsourced topic never reaches the model: {seen:?}"
        );
        assert_eq!(
            vec!["topic-alpha"],
            sitting
                .topics()
                .iter()
                .map(|t| t.topic_id.as_str())
                .collect::<Vec<_>>()
        );

        let bare = workspace_with(TWO_TOPICS);
        let deck = opened(&bare);
        assert!(!deck.declares_sources(), "no manifest, no exam");
        let cli_dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(cli_dir.path(), &[IDEAS]);
        let sitting = Sitting::start(
            &deck,
            SittingStart::default(),
            Strictness::Balanced,
            ExamConfig::default(),
            ask_config(&cli),
        );
        assert_eq!(
            Some("the deck declares no sources to examine against"),
            sitting.error()
        );
        assert_eq!(0, crate::testutil::fake_reply_calls(cli_dir.path()));
    }

    #[test]
    fn a_malformed_sources_manifest_is_a_loud_error_never_cached_as_no_exam() {
        let _lock = exec_lock();
        let ws = two_sourced_topics();
        let manifest = ws.dir().join(sources::MANIFEST);
        let valid = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(&manifest, "version = 1\n[sources\n").unwrap();

        let deck = Deck::load(ws.deck()).unwrap();
        assert!(
            deck.load_warnings
                .iter()
                .any(|w| w.contains(&manifest.display().to_string())),
            "the parse failure is a load warning naming the manifest: {:?}",
            deck.load_warnings
        );
        assert!(
            deck.declares_sources(),
            "a broken manifest is never `no exam`"
        );
        let sitting = Sitting::start(
            &deck,
            SittingStart::default(),
            Strictness::Balanced,
            ExamConfig::default(),
            AskConfig::default(),
        );
        assert!(
            sitting
                .error()
                .is_some_and(|e| e.contains(&manifest.display().to_string())),
            "the sitting fails naming the manifest: {:?}",
            sitting.error()
        );

        let mut cache = crate::cache::DeckCache::default();
        assert!(cache.load(&ws.deck()).unwrap().declares_sources());
        assert!(
            cache.load(&ws.deck()).unwrap().declared().manifest.is_err(),
            "the cache keeps the error, not an empty answer"
        );
        std::fs::write(&manifest, &valid).unwrap();
        assert!(
            cache.load(&ws.deck()).unwrap().declared().manifest.is_ok(),
            "a repaired manifest is read again"
        );
    }

    #[test]
    fn a_ranged_binding_sends_only_its_lines_under_a_range_header() {
        let _lock = exec_lock();
        let ws = workspace_with(TWO_TOPICS);
        let nine: String = (1..=9).map(|n| format!("line-{n}-marker\n")).collect();
        source(&ws, "nine.txt", &nine, &["2-3,7|topic-alpha"]);
        let cli_dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(cli_dir.path(), &[IDEAS, ONE_QUESTION]);
        generate_questions(&opened(&ws), &ExamConfig::default(), &ask_config(&cli)).unwrap();
        for prompt in prompts(cli_dir.path()) {
            assert!(
                prompt.contains(
                    "Source `sdk:nine.txt:2-3`:\n<<<SOURCE\nline-2-marker\nline-3-marker\nSOURCE\n"
                ) && prompt
                    .contains("Source `sdk:nine.txt:7`:\n<<<SOURCE\nline-7-marker\nSOURCE\n"),
                "each range under its own header, lines verbatim: {prompt}"
            );
            for absent in [1, 4, 5, 6, 8, 9] {
                assert!(
                    !prompt.contains(&format!("line-{absent}-marker")),
                    "line {absent} is outside the ranges: {prompt}"
                );
            }
        }
    }

    #[test]
    fn changing_one_topics_ranges_regenerates_only_that_topic() {
        let _lock = exec_lock();
        let ws = workspace_with(TWO_TOPICS);
        let nine: String = (1..=9).map(|n| format!("line-{n}\n")).collect();
        source(&ws, "nine.txt", &nine, &["2-3|topic-alpha", "topic-beta"]);
        prepare(&opened(&ws), &[IDEAS, ONE_QUESTION, IDEAS, ONE_QUESTION]);
        let id = sources::source_id(nine.as_bytes());
        sources::commands::bind(&ws.deck(), &id, &["5-6|topic-alpha".to_string()]).unwrap();
        let (_cli, seen) = prepare(&opened(&ws), &[IDEAS, ONE_QUESTION]);
        assert_eq!(2, seen.len(), "one listing and one question call: {seen:?}");
        assert!(
            seen.iter()
                .all(|p| p.contains("Topic: Alpha\n") && p.contains("sdk:nine.txt:5-6")),
            "both calls are Alpha's, over its new range: {seen:?}"
        );
    }

    #[test]
    fn a_cardless_sourced_topic_is_examined_through_start_with() {
        let _lock = exec_lock();
        let ws = workspace_with("# Empty\n\nOnly prose.\n<!-- id: topic-empty -->\n");
        source(&ws, "empty.txt", "EMPTY_SOURCE\n", &["topic-empty"]);
        let deck = opened(&ws);
        assert_eq!(
            vec!["topic-empty"],
            crate::exam_status::examined_topics(&deck)
                .iter()
                .map(|topic| topic.topic_id.as_str())
                .collect::<Vec<_>>(),
            "the prepared exam's topic list keeps the topic without cards"
        );
        let sources = deck.sources().unwrap().unwrap();
        let cli_dir = tempfile::tempdir().unwrap();
        let cli = crate::testutil::fake_replies(cli_dir.path(), &[IDEAS, ONE_QUESTION]);
        let mut sitting = Sitting::start_with(
            &deck,
            exam_topics(&deck, Some(&sources.manifest)),
            sources,
            None,
            SittingStart::default(),
            Strictness::Balanced,
            ExamConfig::default(),
            ask_config(&cli),
        );
        advance_until_idle(&mut sitting);
        assert_eq!(None, sitting.error());
        assert_eq!(
            vec![("topic-empty", 0)],
            sitting
                .questions()
                .iter()
                .map(|q| (sitting.topics()[q.topic].topic_id.as_str(), q.topic))
                .collect::<Vec<_>>(),
            "the topic without cards is asked about"
        );
    }

    #[test]
    fn a_sitting_refuses_a_deck_whose_topic_id_is_duplicated() {
        let _lock = exec_lock();
        let in_deck = two_sourced_topics();
        let text = std::fs::read_to_string(in_deck.deck()).unwrap();
        std::fs::write(
            in_deck.deck(),
            format!("{text}\n# Gamma\n<!-- id: topic-beta -->\n\n## g?\nthree\n"),
        )
        .unwrap();
        let across = two_sourced_topics();
        std::fs::write(
            across.root().join("decks").join("other.md"),
            "---\nid: \"deck-other\"\n---\n# Copy\n<!-- id: topic-alpha -->\n\n## c?\nx\n",
        )
        .unwrap();
        for (label, ws, copy) in [
            ("in one deck", &in_deck, "d.md"),
            ("across member decks", &across, "other.md"),
        ] {
            let deck = Deck::load(ws.deck()).unwrap();
            let sources = sources::load_verified(ws.root(), DECK).unwrap().unwrap();
            let cli_dir = tempfile::tempdir().unwrap();
            let cli = crate::testutil::fake_replies(cli_dir.path(), &[IDEAS, ONE_QUESTION]);
            let sitting = Sitting::start_with(
                &deck,
                examined_topics(&deck),
                sources,
                None,
                SittingStart::default(),
                Strictness::Balanced,
                ExamConfig::default(),
                ask_config(&cli),
            );
            let findings = crate::topic::duplicated_topics(&deck, Some(ws.root()))
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                !findings.is_empty() && findings.contains(copy),
                "{label}: setup names the copy: {findings}"
            );
            assert_eq!(Some(findings.as_str()), sitting.error(), "{label}: sitting");
            let (rx, _job) = spawn_prepare(
                &deck,
                examined_topics(&deck),
                ExamConfig::default(),
                ask_config(&cli),
            );
            assert_eq!(
                Err(findings.clone()),
                rx.recv().unwrap(),
                "{label}: prepare"
            );
            assert_eq!(
                0,
                crate::testutil::fake_reply_calls(cli_dir.path()),
                "{label}: the model is never called"
            );
        }
    }

    #[test]
    fn narrowing_a_prepared_exam_keeps_only_the_questions_a_sitting_kept() {
        let topics = vec![one_topic()];
        let question = |prompt: &str| ExamQuestion {
            prompt: prompt.to_string(),
            points: vec!["p".to_string()],
            ..Default::default()
        };
        let all = [question("Q1"), question("Q2")];
        let prepared = Prepared {
            version: 1,
            generated_at_ms: 7,
            topics: BTreeMap::new(),
            questions: all
                .iter()
                .filter_map(|q| PreparedQuestion::of(q, &topics))
                .collect(),
            answers: vec![String::new(); 2],
        };
        let narrowed = prepared.narrowed(&topics, &all[1..], &["kept".to_string()]);
        assert_eq!(
            vec![("topic-moves", "Q2")],
            narrowed
                .questions
                .iter()
                .map(|q| (q.topic.as_str(), q.prompt.as_str()))
                .collect::<Vec<_>>()
        );
        assert_eq!(vec!["kept".to_string()], narrowed.answers);
        assert_eq!(7, narrowed.generated_at_ms, "the rest is kept");
    }

    #[test]
    fn a_sitting_records_only_the_topics_it_asked_about() {
        let topic = |id: &str, fingerprint: &str| ExamTopic {
            topic_id: id.to_string(),
            fingerprint: fingerprint.to_string(),
            ..Default::default()
        };
        let topics = [topic("topic-a", "fp-a"), topic("topic-b", "fp-b")];
        let score = |total, pass| TopicScore {
            pass,
            partial: 0,
            total,
            passed: total > 0 && pass == total,
        };
        let result = ExamResult {
            grades: Vec::new(),
            topics: vec![score(1, 1), score(0, 0)],
        };
        let record = sitting_record(&topics, false, &result);
        assert_eq!(
            vec![("topic-a", "fp-a", true)],
            record
                .topics
                .iter()
                .map(|t| (t.topic_id.as_str(), t.fingerprint.as_str(), t.pass))
                .collect::<Vec<_>>(),
            "a topic without a question records nothing"
        );
    }

    fn graduated_pass_at(now_ms: u64) -> crate::store::CardState {
        crate::store::CardState {
            recall: Some(crate::store::FsrsState {
                state: 2,
                scheduled_days: 5,
                due_ms: now_ms + 86_400_000,
                ..Default::default()
            }),
            history: vec![crate::store::Review {
                ts_ms: now_ms.saturating_sub(1),
                grade: crate::scheduler::Grade::Pass,
                depth: crate::depth::Depth::Recall,
                propagated: false,
                confidence: None,
            }],
            ..crate::store::CardState::new()
        }
    }

    fn pass_sitting(deck: &Deck, store: &mut Store, cli: &std::path::Path, now_ms: u64) -> Sitting {
        let start = sitting_start(
            deck,
            store,
            now_ms,
            Some(crate::session::DEFAULT_RETIRE_AFTER_DAYS),
        );
        let mut s = Sitting::start(
            deck,
            start,
            Strictness::Balanced,
            ExamConfig::default(),
            ask_config(cli),
        );
        drain(&mut s, store);
        s.set_answer("a".to_string());
        s.next();
        s.set_answer("b".to_string());
        s.submit();
        drain(&mut s, store);
        assert_eq!(&Phase::Results, s.phase(), "{:?}", s.error());
        assert!(s.result().unwrap().count().passed, "the fake grades pass");
        s
    }

    #[test]
    fn a_passing_sitting_on_an_already_passed_deck_replaces_the_stamp() {
        let _lock = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = branching_cli(
            dir.path(),
            "{\"grades\":[{\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]},\
             {\"verdict\":\"pass\",\"feedback\":\"ok\",\"missed\":[]}]}",
        );
        let ws = workspace_with(
            "# A\n<!-- id: topic-a -->\n\n## a1?\none\n<!-- id: card-a1 -->\n\n\
             # U\n<!-- id: topic-u -->\n\n## u1?\ntwo\n<!-- id: card-u1 -->\n",
        );
        source(&ws, "alpha.txt", "ALPHA_SOURCE\n", &["topic-a"]);
        let now = 1_000_000;
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        *store.get_or_insert("card-u1") = graduated_pass_at(now);
        let deck = opened(&ws);

        let first = pass_sitting(&deck, &mut store, &cli, now);
        assert!(first.eligible(), "sitting 1: u1 meets the condition");
        assert!(first.unlocked(), "sitting 1 set passed_at_ms");
        let stamp_1 = store
            .exam_stamp(DECK)
            .cloned()
            .expect("sitting 1 stamped the deck");
        assert_eq!(vec!["card-u1".to_string()], stamp_1.unsourced_cards);

        let text = std::fs::read_to_string(ws.deck()).unwrap();
        std::fs::write(
            ws.deck(),
            format!("{text}\n## u2?\nthree\n<!-- id: card-u2 -->\n"),
        )
        .unwrap();
        let deck = opened(&ws);
        assert!(
            crate::exam_status::exam_row(&deck, &store, Some(&[])).exam_changed,
            "a card added to an unsourced topic dots the standing"
        );

        let control = pass_sitting(&deck, &mut store, &cli, now);
        assert!(
            !control.eligible(),
            "control: the unscheduled u2 fails the condition"
        );
        assert!(
            !control.unlocked(),
            "control: an already passed deck unlocks nothing"
        );
        assert_eq!(
            Some(&stamp_1),
            store.exam_stamp(DECK),
            "control: an ineligible sitting leaves the stamp"
        );

        *store.get_or_insert("card-u2") = graduated_pass_at(now);
        let second = pass_sitting(&deck, &mut store, &cli, now);
        assert!(
            second.eligible(),
            "sitting 2: both unsourced cards meet the condition"
        );
        assert!(!second.unlocked(), "sitting 2: the deck was already passed");
        assert_eq!(
            Some(second.stamp()),
            store.exam_stamp(DECK),
            "sitting 2 replaces the stamp although passed_at_ms was already set"
        );
        assert_eq!(
            vec!["card-u1".to_string(), "card-u2".to_string()],
            second.stamp().unsourced_cards,
            "the new stamp lists the added card"
        );
        assert!(
            !crate::exam_status::exam_row(&deck, &store, Some(&[])).exam_changed,
            "the standing is solid again"
        );
    }
}
