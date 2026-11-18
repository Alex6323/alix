use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    config::Strictness,
    exam_ideas::IdeasDocument,
    render::{ContentUnit, exam_text_units},
};

pub const VERSION: u32 = 1;
pub const GRADE_REQUEST_VERSION: u32 = 1;
pub const HELD_EXAM_VERSION: u32 = 1;
pub const MAX_ANSWERS_BYTES: usize = 256 * 1024;
pub const MAX_QUESTIONS_REQUEST_BYTES: usize = 2 * 1024 * 1024;
pub const ANSWER_TOO_LONG: &str = "this answer is too long to send";
pub const EXAM_TOO_LARGE: &str = "the exam is too large to hand to a phone";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarriedExam {
    pub version: u32,
    pub sitting: String,
    pub deck: String,
    pub strictness: Strictness,
    pub full: bool,
    pub unknown_cards: Vec<String>,
    pub topics: Vec<CarriedTopic>,
    pub questions: Vec<CarriedQuestion>,
    pub ideas: IdeasDocument,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarriedTopic {
    pub topic_id: String,
    pub fingerprint: String,
    pub label: String,
    pub cards: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarriedQuestion {
    pub topic_id: String,
    pub idea: usize,
    pub prompt: String,
    pub points: Vec<String>,
    pub shape: Shape,
    pub mental_model: String,
}

impl CarriedExam {
    pub fn parse(text: &str) -> Result<Self> {
        let exam: Self = serde_json::from_str(text).context("unreadable carried exam")?;
        exam.check()?;
        Ok(exam)
    }

    pub fn load(path: &Path) -> Result<Option<Self>> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text)
                .with_context(|| format!("reading {}", path.display()))
                .map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self).context("encoding the carried exam")?;
        crate::exam_ideas::write_atomic(path, json.as_bytes())
    }

    pub fn asked_topics(&self) -> Vec<&CarriedTopic> {
        let asked: HashSet<&str> = self.questions.iter().map(|q| q.topic_id.as_str()).collect();
        self.topics
            .iter()
            .filter(|topic| asked.contains(topic.topic_id.as_str()))
            .collect()
    }

    pub fn questions_units(&self) -> Vec<Vec<ContentUnit>> {
        self.questions
            .iter()
            .map(|question| exam_text_units(&question.prompt))
            .collect()
    }

    pub fn grade_request(&self, answers: &[String]) -> GradeRequest {
        let asked = self.asked_topics();
        let number_of = |topic_id: &str| {
            asked
                .iter()
                .position(|topic| topic.topic_id == topic_id)
                .unwrap_or_default()
        };
        GradeRequest {
            version: GRADE_REQUEST_VERSION,
            sitting: self.sitting.clone(),
            deck: self.deck.clone(),
            strictness: self.strictness,
            full: self.full,
            questions: self
                .questions
                .iter()
                .map(|question| GradeQuestion {
                    topic: number_of(&question.topic_id),
                    prompt: question.prompt.clone(),
                    points: question.points.clone(),
                })
                .collect(),
            answers: answers
                .iter()
                .map(|answer| {
                    if answer.trim().is_empty() {
                        String::new()
                    } else {
                        answer.clone()
                    }
                })
                .collect(),
        }
    }

    pub fn check_size(&self) -> Result<()> {
        let blanks = vec![String::new(); self.questions.len()];
        if self.grade_request(&blanks).to_json()?.len() > MAX_QUESTIONS_REQUEST_BYTES {
            bail!(EXAM_TOO_LARGE);
        }
        Ok(())
    }

    fn check(&self) -> Result<()> {
        if self.version != VERSION {
            bail!("unreadable carried exam: version {}", self.version);
        }
        if !self.ideas.has_current_version() {
            bail!(
                "unreadable carried exam: ideas document version {}",
                self.ideas.version
            );
        }
        let mut ids = HashSet::new();
        for topic in &self.topics {
            if !ids.insert(topic.topic_id.as_str()) {
                bail!(
                    "unreadable carried exam: topic {} appears twice",
                    topic.topic_id
                );
            }
        }
        for question in &self.questions {
            if !ids.contains(question.topic_id.as_str()) {
                bail!(
                    "unreadable carried exam: a question names unknown topic {}",
                    question.topic_id
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradeRequest {
    pub version: u32,
    pub sitting: String,
    pub deck: String,
    pub strictness: Strictness,
    pub full: bool,
    pub questions: Vec<GradeQuestion>,
    pub answers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradeQuestion {
    pub topic: usize,
    pub prompt: String,
    pub points: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemediationRequest {
    pub sitting: String,
    pub deck: String,
    pub topics: Vec<RemediationTopic>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemediationTopic {
    pub topic: usize,
    pub gaps: Vec<String>,
}

pub fn answers_bytes(answers: &[String]) -> usize {
    answers.iter().map(|answer| answer.len()).sum()
}

impl GradeRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let request: Self = serde_json::from_slice(bytes).context("unreadable grade request")?;
        request.check()?;
        Ok(request)
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).context("encoding the grade request")
    }

    pub fn topic_count(&self) -> usize {
        self.distinct_topics().len()
    }

    fn distinct_topics(&self) -> HashSet<usize> {
        self.questions
            .iter()
            .map(|question| question.topic)
            .collect()
    }

    fn check(&self) -> Result<()> {
        if self.version != GRADE_REQUEST_VERSION {
            bail!("the grade request is version {}", self.version);
        }
        if self.questions.is_empty() {
            bail!("the grade request has no question");
        }
        if self.answers.len() != self.questions.len() {
            bail!(
                "the grade request has {} answers for {} questions",
                self.answers.len(),
                self.questions.len()
            );
        }
        let topics = self.distinct_topics();
        let largest_plus_one = topics
            .iter()
            .max()
            .and_then(|largest| largest.checked_add(1));
        if largest_plus_one != Some(topics.len()) {
            bail!("the grade request topics are not numbered from 0 without a gap");
        }
        if answers_bytes(&self.answers) > MAX_ANSWERS_BYTES {
            bail!(ANSWER_TOO_LONG);
        }
        Ok(())
    }
}

impl RemediationRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let request: Self =
            serde_json::from_slice(bytes).context("unreadable remediation request")?;
        if request.topics.is_empty() {
            bail!("the remediation request has no topic");
        }
        if request.topics.iter().any(|topic| topic.gaps.is_empty()) {
            bail!("the remediation request has a topic without a gap");
        }
        Ok(request)
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).context("encoding the remediation request")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verdicts {
    pub pass: u32,
    pub partial: u32,
    pub total: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradedAnswer {
    pub topic: usize,
    pub verdict: String,
    pub feedback: String,
    pub feedback_units: Vec<ContentUnit>,
    pub missed: Vec<String>,
    pub missed_units: Vec<Vec<ContentUnit>>,
    pub points_units: Vec<Vec<ContentUnit>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradedTopic {
    pub pass: bool,
    pub verdicts: Verdicts,
    pub gaps: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopicNumberCards {
    pub topic: usize,
    pub cards: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradingReply {
    pub sitting: Option<String>,
    pub phase: String,
    pub grades: Vec<GradedAnswer>,
    pub topics: Vec<GradedTopic>,
    pub verdicts: Option<Verdicts>,
    pub mastered: bool,
    pub cards_by_topic: Vec<TopicNumberCards>,
    pub thinking: bool,
    pub elapsed: Option<u64>,
    pub error: Option<String>,
    pub estimate: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldStart {
    pub eligible: bool,
    pub sourced: String,
    pub unsourced_cards: Vec<String>,
    pub unsourced_topics: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldResult {
    pub received_ms: u64,
    pub recorded: bool,
    pub grades: Vec<GradedAnswer>,
    pub topics: Vec<GradedTopic>,
    pub verdicts: Verdicts,
    pub mastered: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldStanding {
    pub text: String,
    pub exam_changed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldOutcome {
    pub recorded: bool,
    pub standing: Option<HeldStanding>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldExam {
    pub version: u32,
    pub exam: CarriedExam,
    pub questions_units: Vec<Vec<ContentUnit>>,
    pub start: HeldStart,
    pub answers: Vec<String>,
    pub locked: Vec<bool>,
    pub result: Option<HeldResult>,
    pub outcome: Option<HeldOutcome>,
    pub remediated: BTreeMap<String, u32>,
}

fn check_verdicts(grades: &[GradedAnswer]) -> Result<()> {
    let known = [Verdict::Pass, Verdict::Partial, Verdict::Fail].map(Verdict::label);
    match grades
        .iter()
        .find(|grade| !known.contains(&grade.verdict.as_str()))
    {
        Some(grade) => bail!("unknown verdict {}", grade.verdict),
        None => Ok(()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceivedExam {
    exam: CarriedExam,
    questions_units: Vec<Vec<ContentUnit>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HeldView {
    pub sitting: String,
    pub deck: String,
    pub strictness: Strictness,
    pub full: bool,
    pub questions: Vec<String>,
    pub questions_units: Vec<Vec<ContentUnit>>,
    pub answers: Vec<String>,
    pub locked: Vec<bool>,
    pub unsourced_topics: Vec<String>,
    pub outcome: Option<HeldOutcome>,
    pub remediated: BTreeMap<String, u32>,
    pub can_remediate: bool,
    pub result: Option<JoinedResult>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JoinedResult {
    pub grades: Vec<JoinedGrade>,
    pub topics: Vec<JoinedTopic>,
    pub verdicts: Verdicts,
    pub mastered: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JoinedGrade {
    pub topic: usize,
    pub shape: &'static str,
    pub mental_model: String,
    pub question: String,
    pub question_units: Vec<ContentUnit>,
    pub points: Vec<String>,
    pub points_units: Vec<Vec<ContentUnit>>,
    pub answer: String,
    pub verdict: String,
    pub feedback: String,
    pub feedback_units: Vec<ContentUnit>,
    pub missed: Vec<String>,
    pub missed_units: Vec<Vec<ContentUnit>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JoinedTopic {
    pub topic_id: String,
    pub fingerprint: String,
    pub label: String,
    pub cards: Vec<String>,
    pub pass: bool,
    pub verdicts: Verdicts,
}

impl HeldExam {
    pub fn received(carried_json: &str, start: HeldStart) -> Result<Self> {
        let received: ReceivedExam =
            serde_json::from_str(carried_json).context("unreadable carried exam")?;
        received.exam.check()?;
        let count = received.exam.questions.len();
        if received.questions_units.len() != count {
            bail!(
                "unreadable carried exam: {} question units for {count} questions",
                received.questions_units.len()
            );
        }
        Ok(Self {
            version: HELD_EXAM_VERSION,
            exam: received.exam,
            questions_units: received.questions_units,
            start,
            answers: vec![String::new(); count],
            locked: vec![false; count],
            result: None,
            outcome: None,
            remediated: BTreeMap::new(),
        })
    }

    pub fn create(&self, path: &Path) -> Result<()> {
        if path.exists() {
            bail!("an exam is already held for this deck");
        }
        self.save(path)
    }

    pub fn load(path: &Path) -> Result<Option<Self>> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text)
                .with_context(|| format!("reading {}", path.display()))
                .map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.check()
            .context("refusing to save an unreadable held exam")?;
        let json = serde_json::to_string_pretty(self).context("encoding the held exam")?;
        crate::exam_ideas::write_atomic(path, json.as_bytes())
    }

    fn parse(text: &str) -> Result<Self> {
        let held: Self = serde_json::from_str(text).context("unreadable held exam")?;
        held.check().context("unreadable held exam")?;
        Ok(held)
    }

    fn check(&self) -> Result<()> {
        if self.version != HELD_EXAM_VERSION {
            bail!("version {}", self.version);
        }
        self.exam.check()?;
        let count = self.exam.questions.len();
        for (name, length) in [
            ("answers", self.answers.len()),
            ("locks", self.locked.len()),
            ("question units", self.questions_units.len()),
        ] {
            if length != count {
                bail!("{length} {name} for {count} questions");
            }
        }
        if let Some(result) = &self.result {
            self.check_counts(result.grades.len(), result.topics.len())?;
            check_verdicts(&result.grades)?;
        }
        let recorded = match (&self.result, &self.outcome) {
            (None, Some(_)) => bail!("an outcome without a result"),
            (Some(result), Some(outcome)) => {
                if outcome.recorded && !result.recorded {
                    bail!("a recorded outcome for a result not chosen to be recorded");
                }
                outcome.recorded
            }
            (_, None) => false,
        };
        if !recorded && !self.remediated.is_empty() {
            bail!("remediated topics without a recorded outcome");
        }
        Ok(())
    }

    fn check_counts(&self, grades: usize, topics: usize) -> Result<()> {
        let questions = self.exam.questions.len();
        if grades != questions {
            bail!("{grades} grades for {questions} questions");
        }
        let asked = self.exam.asked_topics().len();
        if topics != asked {
            bail!("{topics} graded topics for {asked} asked topics");
        }
        Ok(())
    }

    pub fn set_answers(&mut self, answers: Vec<String>, locked: Vec<bool>) -> Result<()> {
        self.refuse_once_graded()?;
        let count = self.exam.questions.len();
        if answers.len() != count || locked.len() != count {
            bail!(
                "{} answers and {} locks for {count} questions",
                answers.len(),
                locked.len()
            );
        }
        let sent: Vec<String> = answers
            .iter()
            .zip(&locked)
            .filter(|(_, locked)| **locked)
            .map(|(answer, _)| {
                if answer.trim().is_empty() {
                    String::new()
                } else {
                    answer.clone()
                }
            })
            .collect();
        if answers_bytes(&sent) > MAX_ANSWERS_BYTES {
            bail!(ANSWER_TOO_LONG);
        }
        self.answers = answers;
        self.locked = locked;
        Ok(())
    }

    pub fn grade_request(&self) -> Result<GradeRequest> {
        self.refuse_once_graded()?;
        let request = self.exam.grade_request(&self.answers);
        if answers_bytes(&request.answers) > MAX_ANSWERS_BYTES {
            bail!(ANSWER_TOO_LONG);
        }
        Ok(request)
    }

    fn refuse_once_graded(&self) -> Result<()> {
        if self.result.is_some() {
            bail!("this exam was already graded");
        }
        Ok(())
    }

    pub fn store_result(
        &mut self,
        reply: &GradingReply,
        received_ms: u64,
        recorded: bool,
    ) -> Result<()> {
        self.refuse_once_graded()?;
        if reply.sitting.as_deref() != Some(self.exam.sitting.as_str()) {
            bail!("the grading reply is for another sitting");
        }
        if reply.phase != "results" {
            bail!("the grading reply is in phase {}", reply.phase);
        }
        self.check_counts(reply.grades.len(), reply.topics.len())
            .context("the grading reply does not fit the exam")?;
        let request = self.exam.grade_request(&self.answers);
        let numbered_alike = reply
            .grades
            .iter()
            .zip(&request.questions)
            .all(|(grade, question)| grade.topic == question.topic);
        if !numbered_alike {
            bail!("the grading reply numbers a grade's topic unlike its question");
        }
        check_verdicts(&reply.grades)?;
        let Some(verdicts) = reply.verdicts else {
            bail!("the grading reply has no verdicts");
        };
        self.result = Some(HeldResult {
            received_ms,
            recorded,
            grades: reply.grades.clone(),
            topics: reply.topics.clone(),
            verdicts,
            mastered: reply.mastered,
        });
        Ok(())
    }

    pub fn asked(&self) -> Vec<crate::exam_ideas::AskedIdea> {
        let Some(result) = &self.result else {
            return Vec::new();
        };
        self.exam
            .questions
            .iter()
            .zip(&result.grades)
            .map(|(question, grade)| crate::exam_ideas::AskedIdea {
                topic: question.topic_id.clone(),
                idea: question.idea,
                passed: grade.verdict == Verdict::Pass.label(),
            })
            .collect()
    }

    pub fn sitting_record(&self) -> Option<crate::store::SittingRecord> {
        let result = self.result.as_ref()?;
        Some(crate::store::SittingRecord {
            topics: self
                .exam
                .asked_topics()
                .iter()
                .zip(&result.topics)
                .map(|(topic, graded)| crate::store::TopicRecord {
                    topic_id: topic.topic_id.clone(),
                    fingerprint: topic.fingerprint.clone(),
                    pass: graded.pass,
                })
                .collect(),
            full: self.exam.full,
            pass: result.verdicts.pass,
            partial: result.verdicts.partial,
            total: result.verdicts.total,
        })
    }

    pub fn remediation_request(&self) -> Option<RemediationRequest> {
        let recorded = self
            .outcome
            .as_ref()
            .is_some_and(|outcome| outcome.recorded);
        let result = self.result.as_ref().filter(|_| recorded)?;
        let topics: Vec<RemediationTopic> = self
            .exam
            .asked_topics()
            .iter()
            .zip(&result.topics)
            .enumerate()
            .filter(|(_, (topic, graded))| {
                !graded.pass
                    && !graded.gaps.is_empty()
                    && !self.remediated.contains_key(&topic.topic_id)
            })
            .map(|(number, (_, graded))| RemediationTopic {
                topic: number,
                gaps: graded.gaps.clone(),
            })
            .collect();
        if topics.is_empty() {
            return None;
        }
        Some(RemediationRequest {
            sitting: self.exam.sitting.clone(),
            deck: self.exam.deck.clone(),
            topics,
        })
    }

    pub fn topic_id_of(&self, topic: usize) -> Option<&str> {
        self.exam
            .asked_topics()
            .get(topic)
            .map(|asked| asked.topic_id.as_str())
    }

    pub fn view(&self) -> HeldView {
        HeldView {
            sitting: self.exam.sitting.clone(),
            deck: self.exam.deck.clone(),
            strictness: self.exam.strictness,
            full: self.exam.full,
            questions: self
                .exam
                .questions
                .iter()
                .map(|question| question.prompt.clone())
                .collect(),
            questions_units: self.questions_units.clone(),
            answers: self.answers.clone(),
            locked: self.locked.clone(),
            unsourced_topics: self.start.unsourced_topics.clone(),
            outcome: self.outcome.clone(),
            remediated: self.remediated.clone(),
            can_remediate: self.remediation_request().is_some(),
            result: self.result.as_ref().map(|result| self.joined(result)),
        }
    }

    fn joined(&self, result: &HeldResult) -> JoinedResult {
        let request = self.exam.grade_request(&self.answers);
        let grades = self
            .exam
            .questions
            .iter()
            .zip(&request.questions)
            .zip(&self.questions_units)
            .zip(&self.answers)
            .zip(&result.grades)
            .map(
                |((((question, numbered), question_units), answer), grade)| JoinedGrade {
                    topic: numbered.topic,
                    shape: question.shape.label(),
                    mental_model: question.mental_model.clone(),
                    question: question.prompt.clone(),
                    question_units: question_units.clone(),
                    points: question.points.clone(),
                    points_units: grade.points_units.clone(),
                    answer: answer.clone(),
                    verdict: grade.verdict.clone(),
                    feedback: grade.feedback.clone(),
                    feedback_units: grade.feedback_units.clone(),
                    missed: grade.missed.clone(),
                    missed_units: grade.missed_units.clone(),
                },
            )
            .collect();
        let topics = self
            .exam
            .asked_topics()
            .iter()
            .zip(&result.topics)
            .map(|(topic, graded)| JoinedTopic {
                topic_id: topic.topic_id.clone(),
                fingerprint: topic.fingerprint.clone(),
                label: topic.label.clone(),
                cards: topic.cards.clone(),
                pass: graded.pass,
                verdicts: graded.verdicts,
            })
            .collect();
        JoinedResult {
            grades,
            topics,
            verdicts: result.verdicts,
            mastered: result.mastered,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::render::exam_text_units;

    fn topic(id: &str) -> CarriedTopic {
        CarriedTopic {
            topic_id: id.into(),
            fingerprint: format!("fp-{id}"),
            label: format!("Topic {id}"),
            cards: vec![format!("card-{id}")],
        }
    }

    fn question(topic_id: &str) -> CarriedQuestion {
        CarriedQuestion {
            topic_id: topic_id.into(),
            idea: 0,
            prompt: format!("Why `{topic_id}`?"),
            points: vec!["a point".into()],
            shape: Shape::Boundary,
            mental_model: "a model".into(),
        }
    }

    fn exam(topics: &[&str], asked: &[&str]) -> CarriedExam {
        CarriedExam {
            version: VERSION,
            sitting: "s1".into(),
            deck: "deck-1".into(),
            strictness: Strictness::default(),
            full: true,
            unknown_cards: vec!["card-x".into()],
            topics: topics.iter().map(|id| topic(id)).collect(),
            questions: asked.iter().map(|id| question(id)).collect(),
            ideas: IdeasDocument {
                version: crate::exam_ideas::VERSION,
                topics: BTreeMap::new(),
            },
        }
    }

    fn with_field(exam: &CarriedExam, edit: impl FnOnce(&mut serde_json::Value)) -> String {
        let mut value = serde_json::to_value(exam).unwrap();
        edit(&mut value);
        value.to_string()
    }

    fn assert_unreadable(text: &str) {
        let error = CarriedExam::parse(text).unwrap_err();
        assert!(
            error.to_string().contains("unreadable carried exam"),
            "error was: {error}"
        );
    }

    #[test]
    fn a_carried_exam_round_trips_through_its_file_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.json");
        let second = dir.path().join("second.json");
        let original = exam(&["a", "b"], &["a"]);
        original.save(&first).unwrap();
        let loaded = CarriedExam::load(&first).unwrap().unwrap();
        loaded.save(&second).unwrap();
        assert_eq!(loaded, original, "loaded value");
        assert_eq!(
            std::fs::read(&first).unwrap(),
            std::fs::read(&second).unwrap(),
            "bytes after a second save"
        );
    }

    #[test]
    fn an_absent_file_loads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = CarriedExam::load(&dir.path().join("none.json")).unwrap();
        assert_eq!(loaded, None);
    }

    #[test]
    fn a_document_of_another_version_is_unreadable() {
        let text = with_field(&exam(&["a"], &[]), |v| v["version"] = 2.into());
        assert_unreadable(&text);
    }

    #[test]
    fn a_document_with_an_unknown_field_is_unreadable() {
        let carried = exam(&["a"], &["a"]);
        for (place, parent) in [
            ("top level", ""),
            ("topics[0]", "/topics/0"),
            ("questions[0]", "/questions/0"),
        ] {
            let text = with_field(&carried, |v| {
                v.pointer_mut(parent).unwrap()["extra"] = true.into();
            });
            let error = CarriedExam::parse(&text)
                .expect_err(&format!("an unknown field at {place} was accepted"));
            assert!(
                error.to_string().contains("unreadable carried exam"),
                "unknown field at {place}: error was: {error}"
            );
        }
    }

    #[test]
    fn two_topics_with_one_id_are_unreadable() {
        let text = serde_json::to_string(&exam(&["a", "a"], &[])).unwrap();
        assert_unreadable(&text);
    }

    #[test]
    fn a_question_naming_no_topic_is_unreadable() {
        let text = serde_json::to_string(&exam(&["a"], &["z"])).unwrap();
        assert_unreadable(&text);
    }

    #[test]
    fn asked_topics_skip_a_topic_without_a_question_and_keep_document_order() {
        let carried = exam(&["a", "b", "c"], &["c", "a"]);
        let ids: Vec<&str> = carried
            .asked_topics()
            .iter()
            .map(|t| t.topic_id.as_str())
            .collect();
        assert_eq!(ids, ["a", "c"]);
    }

    #[test]
    fn questions_units_equal_exam_text_units_of_each_prompt() {
        let carried = exam(&["a", "b"], &["a", "b"]);
        let expected: Vec<_> = carried
            .questions
            .iter()
            .map(|q| exam_text_units(&q.prompt))
            .collect();
        assert_eq!(carried.questions_units(), expected);
    }

    #[test]
    fn a_document_without_the_full_flag_and_topic_labels_is_unreadable() {
        let without_full = with_field(&exam(&["a"], &[]), |v| {
            v.as_object_mut().unwrap().remove("full");
        });
        assert_unreadable(&without_full);
        let without_label = with_field(&exam(&["a"], &[]), |v| {
            v["topics"][0].as_object_mut().unwrap().remove("label");
        });
        assert_unreadable(&without_label);
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn valid_request() -> GradeRequest {
        exam(&["a", "b"], &["a", "b"]).grade_request(&strings(&["x", "y"]))
    }

    fn one_point_exam(point: String) -> CarriedExam {
        let mut carried = exam(&["a"], &["a"]);
        carried.questions[0].points = vec![point];
        carried
    }

    fn request_len(carried: &CarriedExam) -> usize {
        carried
            .grade_request(&[String::new()])
            .to_json()
            .unwrap()
            .len()
    }

    #[test]
    fn the_builder_numbers_topics_in_document_order_among_those_that_drew_a_question() {
        let carried = exam(&["a", "b", "c"], &["c", "a", "c"]);
        let request = carried.grade_request(&strings(&["1", "2", "3"]));
        let numbers: Vec<usize> = request.questions.iter().map(|q| q.topic).collect();
        assert_eq!(numbers, [1, 0, 1], "topic numbers per question");
        assert_eq!(request.topic_count(), 2, "topic count");
    }

    #[test]
    fn the_builder_sends_a_blank_answer_empty_and_leaves_the_others_untouched() {
        let carried = exam(&["a"], &["a", "a"]);
        let request = carried.grade_request(&strings(&["  \n", " x "]));
        assert_eq!(request.answers, strings(&["", " x "]));
    }

    #[test]
    fn a_built_request_parses_back_equal() {
        let built = valid_request();
        let parsed = GradeRequest::parse(built.to_json().unwrap().as_bytes()).unwrap();
        assert_eq!(parsed, built);
    }

    #[test]
    fn two_builds_from_one_exam_and_answers_are_equal_and_one_changed_answer_is_not() {
        let carried = exam(&["a", "b"], &["a", "b"]);
        let answers = strings(&["x", "y"]);
        assert_eq!(
            carried.grade_request(&answers),
            carried.grade_request(&answers),
            "same inputs"
        );
        assert_ne!(
            carried.grade_request(&answers),
            carried.grade_request(&strings(&["x", "z"])),
            "one answer changed"
        );
    }

    #[test]
    fn parse_refuses_each_malformed_request() {
        fn bytes(edit: impl FnOnce(&mut GradeRequest)) -> Vec<u8> {
            let mut request = valid_request();
            edit(&mut request);
            serde_json::to_vec(&request).unwrap()
        }
        let rows: Vec<(&str, Vec<u8>, &str)> = vec![
            ("not json", b"not json".to_vec(), "unreadable grade request"),
            (
                "version 2",
                bytes(|r| r.version = 2),
                "grade request is version",
            ),
            (
                "no question",
                bytes(|r| {
                    r.questions.clear();
                    r.answers.clear();
                }),
                "has no question",
            ),
            (
                "2 questions, 1 answer",
                bytes(|r| r.answers.truncate(1)),
                "answers for",
            ),
            (
                "topics 0 and 2",
                bytes(|r| r.questions[1].topic = 2),
                "not numbered from 0",
            ),
            (
                "topics 1 only",
                bytes(|r| {
                    r.questions.truncate(1);
                    r.answers.truncate(1);
                    r.questions[0].topic = 1;
                }),
                "not numbered from 0",
            ),
            (
                "topic usize::MAX",
                bytes(|r| {
                    r.questions.truncate(1);
                    r.answers.truncate(1);
                    r.questions[0].topic = usize::MAX;
                }),
                "not numbered from 0",
            ),
            (
                "answers one byte over",
                bytes(|r| {
                    r.answers = vec!["x".repeat(MAX_ANSWERS_BYTES), "y".into()];
                }),
                ANSWER_TOO_LONG,
            ),
        ];
        for (name, input, fragment) in rows {
            let error = GradeRequest::parse(&input).expect_err(name);
            assert!(
                error.to_string().contains(fragment),
                "row {name}: expected fragment {fragment:?}, error was: {error}"
            );
        }
    }

    #[test]
    fn answers_of_exactly_the_limit_are_accepted() {
        let mut request = valid_request();
        request.answers = vec!["x".repeat(MAX_ANSWERS_BYTES - 1), "y".into()];
        let parsed = GradeRequest::parse(&serde_json::to_vec(&request).unwrap()).unwrap();
        assert_eq!(answers_bytes(&parsed.answers), MAX_ANSWERS_BYTES);
    }

    #[test]
    fn answers_bytes_counts_utf8_bytes() {
        assert_eq!(answers_bytes(&strings(&["€€"])), 6);
    }

    #[test]
    fn check_size_refuses_an_exam_one_byte_over_and_accepts_one_at_the_limit() {
        let base = request_len(&one_point_exam(String::new()));
        let at_limit = one_point_exam("p".repeat(MAX_QUESTIONS_REQUEST_BYTES - base));
        assert_eq!(request_len(&at_limit), MAX_QUESTIONS_REQUEST_BYTES);
        at_limit.check_size().expect("an exam at the limit");
        let over = one_point_exam("p".repeat(MAX_QUESTIONS_REQUEST_BYTES - base + 1));
        let error = over.check_size().unwrap_err();
        assert_eq!(error.to_string(), EXAM_TOO_LARGE);
    }

    #[test]
    fn a_request_at_both_limits_with_sixfold_escaping_stays_under_four_mebibytes() {
        let escaped = '\u{1}';
        let base = request_len(&one_point_exam(String::new()));
        let fitting = (MAX_QUESTIONS_REQUEST_BYTES - base) / 6;
        let point = |chars: usize| one_point_exam(escaped.to_string().repeat(chars));
        point(fitting).check_size().expect("the largest accepted");
        assert!(point(fitting + 1).check_size().is_err(), "one more char");
        let answers = [escaped.to_string().repeat(MAX_ANSWERS_BYTES)];
        let length = point(fitting)
            .grade_request(&answers)
            .to_json()
            .unwrap()
            .len();
        assert!(
            length <= 4 * 1024 * 1024,
            "the request was {length} bytes, over 4 MiB"
        );
    }

    #[test]
    fn remediation_parse_refuses_no_topic_and_a_topic_without_a_gap() {
        let request = |topics: Vec<RemediationTopic>| {
            serde_json::to_vec(&RemediationRequest {
                sitting: "s1".into(),
                deck: "deck-1".into(),
                topics,
            })
            .unwrap()
        };
        let gapless = RemediationTopic {
            topic: 0,
            gaps: vec![],
        };
        let full = RemediationTopic {
            topic: 0,
            gaps: strings(&["g"]),
        };
        let error = RemediationRequest::parse(&request(vec![])).unwrap_err();
        assert!(error.to_string().contains("no topic"), "no topic: {error}");
        let error = RemediationRequest::parse(&request(vec![full.clone(), gapless])).unwrap_err();
        assert!(
            error.to_string().contains("without a gap"),
            "gapless topic: {error}"
        );
        let parsed = RemediationRequest::parse(&request(vec![full])).unwrap();
        let again = RemediationRequest::parse(parsed.to_json().unwrap().as_bytes()).unwrap();
        assert_eq!(again, parsed, "a valid request round trips");
    }

    #[test]
    fn a_request_struct_with_an_unknown_field_is_unreadable() {
        fn rejects<T: serde::de::DeserializeOwned + Serialize>(value: &T, name: &str) {
            let mut json = serde_json::to_value(value).unwrap();
            json["extra"] = true.into();
            assert!(
                serde_json::from_value::<T>(json).is_err(),
                "{name} accepted an unknown field"
            );
        }
        let request = valid_request();
        rejects(&request, "GradeRequest");
        rejects(&request.questions[0], "GradeQuestion");
        let remediation = RemediationRequest {
            sitting: "s1".into(),
            deck: "deck-1".into(),
            topics: vec![RemediationTopic {
                topic: 0,
                gaps: strings(&["g"]),
            }],
        };
        rejects(&remediation, "RemediationRequest");
        rejects(&remediation.topics[0], "RemediationTopic");
    }

    fn start() -> HeldStart {
        HeldStart {
            eligible: true,
            sourced: "sourced-1".into(),
            unsourced_cards: strings(&["card-u"]),
            unsourced_topics: strings(&["topic-u"]),
        }
    }

    fn received_text(carried: &CarriedExam, units: &[Vec<ContentUnit>]) -> String {
        serde_json::json!({ "exam": carried, "questions_units": units }).to_string()
    }

    fn held_from(carried: CarriedExam) -> HeldExam {
        let units = carried.questions_units();
        HeldExam::received(&received_text(&carried, &units), start()).unwrap()
    }

    fn held() -> HeldExam {
        held_from(exam(&["a", "b", "c"], &["c", "a"]))
    }

    fn units(text: &str) -> Vec<ContentUnit> {
        exam_text_units(text)
    }

    fn graded(topic: usize, verdict: &str, tag: &str) -> GradedAnswer {
        GradedAnswer {
            topic,
            verdict: verdict.into(),
            feedback: format!("feedback {tag}"),
            feedback_units: units(&format!("feedback {tag}")),
            missed: vec![format!("missed {tag}")],
            missed_units: vec![units(&format!("missed {tag}"))],
            points_units: vec![units(&format!("point {tag}"))],
        }
    }

    fn graded_topic(pass: bool, gaps: &[&str]) -> GradedTopic {
        GradedTopic {
            pass,
            verdicts: Verdicts {
                pass: u32::from(pass),
                partial: 0,
                total: 1,
            },
            gaps: strings(gaps),
        }
    }

    fn fitting_reply(held: &HeldExam) -> GradingReply {
        let request = held.exam.grade_request(&held.answers);
        GradingReply {
            sitting: Some(held.exam.sitting.clone()),
            phase: "results".into(),
            grades: request
                .questions
                .iter()
                .enumerate()
                .map(|(i, q)| graded(q.topic, "PASS", &i.to_string()))
                .collect(),
            topics: held
                .exam
                .asked_topics()
                .iter()
                .map(|_| graded_topic(true, &[]))
                .collect(),
            verdicts: Some(Verdicts {
                pass: 2,
                partial: 0,
                total: 2,
            }),
            mastered: true,
            cards_by_topic: vec![TopicNumberCards {
                topic: 0,
                cards: "card-a".into(),
            }],
            thinking: false,
            elapsed: Some(7),
            error: None,
            estimate: None,
        }
    }

    fn graded_held() -> HeldExam {
        let mut value = held();
        value
            .set_answers(strings(&["answer 0", "answer 1"]), vec![true, true])
            .unwrap();
        let reply = fitting_reply(&value);
        value.store_result(&reply, 1000, true).unwrap();
        value
    }

    fn recorded(mut value: HeldExam) -> HeldExam {
        value.outcome = Some(HeldOutcome {
            recorded: true,
            standing: Some(HeldStanding {
                text: "standing".into(),
                exam_changed: false,
            }),
        });
        value
    }

    fn load_text(text: &str) -> Result<Option<HeldExam>> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("held.json");
        std::fs::write(&path, text).unwrap();
        HeldExam::load(&path)
    }

    #[test]
    fn a_received_document_becomes_a_held_exam_with_empty_unlocked_answers() {
        let carried = exam(&["a", "b", "c"], &["b", "a"]);
        let value = held_from(carried.clone());
        assert_eq!(value.version, HELD_EXAM_VERSION, "version");
        assert_eq!(value.exam, carried, "exam");
        assert_eq!(value.questions_units, carried.questions_units(), "units");
        assert_eq!(value.start, start(), "start");
        assert_eq!(value.answers, strings(&["", ""]), "answers");
        assert_eq!(value.locked, [false, false], "locked");
        assert_eq!(value.result, None, "result");
        assert_eq!(value.outcome, None, "outcome");
        assert!(value.remediated.is_empty(), "remediated");
    }

    #[test]
    fn a_received_document_with_a_units_count_unlike_its_questions_is_refused() {
        let carried = exam(&["a", "b"], &["a", "b"]);
        let mut short = carried.questions_units();
        short.pop();
        let error = HeldExam::received(&received_text(&carried, &short), start()).unwrap_err();
        assert!(
            error.to_string().contains("question units"),
            "error was: {error}"
        );
        let units = carried.questions_units();
        HeldExam::received(&received_text(&carried, &units), start())
            .expect("a matching units count");
    }

    #[test]
    fn create_refuses_to_replace_a_held_file_and_leaves_it_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("held.json");
        held().create(&path).expect("the first create");
        let before = std::fs::read(&path).unwrap();
        let other = graded_held();
        let error = other.create(&path).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("an exam is already held for this deck"),
            "error was: {error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before, "bytes after refusal");
    }

    #[test]
    fn a_held_exam_round_trips_through_its_file_in_every_state() {
        let fresh = held();
        let mut answered = held();
        answered
            .set_answers(strings(&["one", ""]), vec![true, false])
            .unwrap();
        let with_result = graded_held();
        let with_outcome = recorded(graded_held());
        let mut with_remediated = recorded(graded_held());
        with_remediated.remediated.insert("a".into(), 3);
        let dir = tempfile::tempdir().unwrap();
        for (state, value) in [
            ("fresh", fresh),
            ("answered", answered),
            ("with result", with_result),
            ("with outcome", with_outcome),
            ("with remediated", with_remediated),
        ] {
            let first = dir.path().join(format!("{state}-1.json"));
            let second = dir.path().join(format!("{state}-2.json"));
            value.save(&first).unwrap();
            let loaded = HeldExam::load(&first)
                .unwrap_or_else(|e| panic!("{state}: load failed: {e:#}"))
                .unwrap_or_else(|| panic!("{state}: loaded as absent"));
            assert_eq!(loaded, value, "{state}: loaded value");
            loaded.save(&second).unwrap();
            assert_eq!(
                std::fs::read(&first).unwrap(),
                std::fs::read(&second).unwrap(),
                "{state}: bytes after a second save"
            );
        }
    }

    #[test]
    fn set_answers_refuses_a_lock_that_takes_the_locked_answers_over_the_limit() {
        let mut value = held();
        let long = strings(&[&"x".repeat(MAX_ANSWERS_BYTES), "y"]);
        let before = value.clone();
        let error = value
            .set_answers(long.clone(), vec![true, true])
            .unwrap_err();
        assert_eq!(error.to_string(), ANSWER_TOO_LONG, "locking both");
        assert_eq!(value, before, "value after the refusal");
        value
            .set_answers(long.clone(), vec![false, false])
            .expect("the same text unlocked");
        value
            .set_answers(long.clone(), vec![true, false])
            .expect("only the first locked, at the limit");
        assert_eq!(value.answers, long, "answers kept");
        assert_eq!(value.locked, [true, false], "locks kept");
    }

    #[test]
    fn three_byte_characters_are_counted_alike_by_the_lock_and_by_parse() {
        let at_limit = format!("{}x", "€".repeat(MAX_ANSWERS_BYTES / 3));
        assert_eq!(at_limit.len(), MAX_ANSWERS_BYTES, "fixture length");
        let over = "€".repeat(MAX_ANSWERS_BYTES / 3 + 1);
        for (name, answer, fits) in [("at the limit", at_limit, true), ("over", over, false)] {
            let mut value = held_from(exam(&["a"], &["a"]));
            let locked = value.set_answers(vec![answer.clone()], vec![true]);
            let request = value.exam.grade_request(&[answer]);
            let parsed = GradeRequest::parse(request.to_json().unwrap().as_bytes());
            assert_eq!(locked.is_ok(), fits, "{name}: the lock");
            assert_eq!(parsed.is_ok(), fits, "{name}: parse");
        }
    }

    #[test]
    fn grade_request_is_refused_once_a_result_is_stored() {
        let mut value = held();
        value.grade_request().expect("before a result");
        let reply = fitting_reply(&value);
        value.store_result(&reply, 1, true).unwrap();
        let error = value.grade_request().unwrap_err();
        assert!(
            error.to_string().contains("this exam was already graded"),
            "error was: {error}"
        );
    }

    #[test]
    fn store_result_keeps_the_time_and_the_choice() {
        for (received_ms, choice) in [(1234, false), (5678, true)] {
            let mut value = held();
            let reply = fitting_reply(&value);
            value.store_result(&reply, received_ms, choice).unwrap();
            let result = value.result.unwrap();
            assert_eq!(result.received_ms, received_ms, "time");
            assert_eq!(result.recorded, choice, "choice at {received_ms}");
            assert_eq!(result.grades, reply.grades, "grades");
            assert_eq!(result.topics, reply.topics, "topics");
            assert_eq!(Some(result.verdicts), reply.verdicts, "verdicts");
            assert_eq!(result.mastered, reply.mastered, "mastered");
        }
    }

    #[test]
    fn a_second_store_result_is_refused_and_the_first_stays() {
        let mut value = held();
        let first = fitting_reply(&value);
        value.store_result(&first, 1, true).unwrap();
        let stored = value.clone();
        let mut second = fitting_reply(&value);
        second.mastered = false;
        assert!(
            value.store_result(&second, 2, false).is_err(),
            "the second store was accepted"
        );
        assert_eq!(value, stored, "value after the second store");
    }

    #[test]
    fn asked_marks_an_idea_passed_only_for_a_pass_verdict() {
        let mut carried = exam(&["a", "b", "c"], &["b", "a", "c"]);
        carried.questions[1].idea = 2;
        carried.questions[2].idea = 5;
        let mut value = held_from(carried);
        assert!(value.asked().is_empty(), "no result");
        let mut reply = fitting_reply(&value);
        for (grade, verdict) in reply.grades.iter_mut().zip(["PASS", "PARTIAL", "FAIL"]) {
            grade.verdict = verdict.into();
        }
        value.store_result(&reply, 1, true).unwrap();
        let idea = |topic: &str, idea, passed| crate::exam_ideas::AskedIdea {
            topic: topic.into(),
            idea,
            passed,
        };
        assert_eq!(
            value.asked(),
            [idea("b", 0, true), idea("a", 2, false), idea("c", 5, false)]
        );
    }

    #[test]
    fn sitting_record_names_each_asked_topic_with_its_fingerprint_and_pass() {
        for full in [true, false] {
            let mut value = held();
            value.exam.full = full;
            assert_eq!(value.sitting_record(), None, "full {full}: no result");
            let mut reply = fitting_reply(&value);
            reply.topics = vec![graded_topic(true, &[]), graded_topic(false, &["g"])];
            reply.verdicts = Some(Verdicts {
                pass: 3,
                partial: 1,
                total: 5,
            });
            value.store_result(&reply, 1, true).unwrap();
            let topic = |id: &str, pass| crate::store::TopicRecord {
                topic_id: id.into(),
                fingerprint: format!("fp-{id}"),
                pass,
            };
            assert_eq!(
                value.sitting_record(),
                Some(crate::store::SittingRecord {
                    topics: vec![topic("a", true), topic("c", false)],
                    full,
                    pass: 3,
                    partial: 1,
                    total: 5,
                }),
                "full {full}"
            );
        }
    }

    #[test]
    fn remediation_request_holds_only_unpassed_topics_with_gaps_not_yet_remediated() {
        let mut value = held_from(exam(&["a", "b", "x", "c", "d"], &["a", "b", "c", "d"]));
        let mut reply = fitting_reply(&value);
        reply.topics = vec![
            graded_topic(true, &["partial gap"]),
            graded_topic(false, &[]),
            graded_topic(false, &["c gap"]),
            graded_topic(false, &["d gap"]),
        ];
        value.store_result(&reply, 1, true).unwrap();
        let mut value = recorded(value);
        value.remediated.insert("c".into(), 2);
        assert_eq!(
            value.remediation_request(),
            Some(RemediationRequest {
                sitting: "s1".into(),
                deck: "deck-1".into(),
                topics: vec![RemediationTopic {
                    topic: 3,
                    gaps: strings(&["d gap"]),
                }],
            })
        );
        value.remediated.insert("d".into(), 1);
        assert_eq!(value.remediation_request(), None, "no topic left");
    }

    #[test]
    fn remediation_request_is_none_without_a_recorded_outcome() {
        let unrecorded = {
            let mut value = recorded(graded_held());
            value.outcome = Some(HeldOutcome {
                recorded: false,
                standing: None,
            });
            value
        };
        let failing = |mut value: HeldExam| {
            if let Some(result) = value.result.as_mut() {
                result.topics[0] = graded_topic(false, &["g"]);
            }
            value
        };
        for (row, value) in [
            ("no result", held()),
            ("a result and no outcome", failing(graded_held())),
            ("an outcome that says not recorded", failing(unrecorded)),
        ] {
            assert_eq!(value.remediation_request(), None, "{row}");
        }
        assert!(
            failing(recorded(graded_held()))
                .remediation_request()
                .is_some(),
            "control: a recorded outcome with a failed topic"
        );
    }

    #[test]
    fn a_reply_that_does_not_fit_the_exam_is_refused_and_the_value_is_unchanged() {
        let value = held();
        type ReplyEdit = fn(&mut GradingReply);
        let rows: Vec<(&str, ReplyEdit)> = vec![
            ("another sitting", |r| r.sitting = Some("s2".into())),
            ("phase grading", |r| r.phase = "grading".into()),
            ("one grade too few", |r| {
                r.grades.pop();
            }),
            ("one topic too many", |r| {
                r.topics.push(graded_topic(true, &[]))
            }),
            ("a grade naming another topic number", |r| {
                r.grades[0].topic = r.grades[1].topic
            }),
            ("no verdicts", |r| r.verdicts = None),
            ("a verdict outside the three labels", |r| {
                r.grades[0].verdict = "pass".into()
            }),
        ];
        for (row, edit) in rows {
            let mut reply = fitting_reply(&value);
            edit(&mut reply);
            let mut changed = value.clone();
            assert!(
                changed.store_result(&reply, 1, true).is_err(),
                "row {row}: the reply was accepted"
            );
            assert_eq!(changed, value, "row {row}: value after the refusal");
        }
        let mut control = value.clone();
        control
            .store_result(&fitting_reply(&value), 1, true)
            .expect("control: the fitting reply");
    }

    #[test]
    fn the_view_joins_each_grade_with_its_question_points_answer_and_topic() {
        let mut carried = exam(&["a", "b", "c"], &["c", "a"]);
        carried.full = false;
        carried.questions[0].shape = Shape::Fix;
        carried.questions[0].mental_model = "model c".into();
        carried.questions[0].points = strings(&["point c"]);
        let mut value = held_from(carried);
        let fresh = serde_json::to_value(value.view()).unwrap();
        assert_eq!(fresh["result"], serde_json::Value::Null, "no result");
        value
            .set_answers(strings(&["answer 0", "answer 1"]), vec![true, false])
            .unwrap();
        let mut reply = fitting_reply(&value);
        reply.grades[1].verdict = "FAIL".into();
        reply.topics = vec![graded_topic(false, &["gap a"]), graded_topic(true, &[])];
        value.store_result(&reply, 1, true).unwrap();
        let value = recorded(value);
        let joined = |i: usize, topic: usize, shape: &'static str| {
            let question = &value.exam.questions[i];
            let grade = &reply.grades[i];
            JoinedGrade {
                topic,
                shape,
                mental_model: question.mental_model.clone(),
                question: question.prompt.clone(),
                question_units: units(&question.prompt),
                points: question.points.clone(),
                points_units: grade.points_units.clone(),
                answer: value.answers[i].clone(),
                verdict: grade.verdict.clone(),
                feedback: grade.feedback.clone(),
                feedback_units: grade.feedback_units.clone(),
                missed: grade.missed.clone(),
                missed_units: grade.missed_units.clone(),
            }
        };
        let topic = |id: &str, graded: &GradedTopic| JoinedTopic {
            topic_id: id.into(),
            fingerprint: format!("fp-{id}"),
            label: format!("Topic {id}"),
            cards: vec![format!("card-{id}")],
            pass: graded.pass,
            verdicts: graded.verdicts,
        };
        let expected = HeldView {
            sitting: "s1".into(),
            deck: "deck-1".into(),
            strictness: Strictness::default(),
            full: false,
            questions: strings(&["Why `c`?", "Why `a`?"]),
            questions_units: value.questions_units.clone(),
            answers: strings(&["answer 0", "answer 1"]),
            locked: vec![true, false],
            unsourced_topics: strings(&["topic-u"]),
            outcome: value.outcome.clone(),
            remediated: BTreeMap::new(),
            can_remediate: true,
            result: Some(JoinedResult {
                grades: vec![joined(0, 1, "fix"), joined(1, 0, "boundary")],
                topics: vec![topic("a", &reply.topics[0]), topic("c", &reply.topics[1])],
                verdicts: reply.verdicts.unwrap(),
                mastered: true,
            }),
        };
        assert_eq!(value.view(), expected);
    }

    #[test]
    fn an_absent_held_file_loads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(HeldExam::load(&dir.path().join("none.json")).unwrap(), None);
    }

    #[test]
    fn a_held_file_that_does_not_fit_its_exam_is_unreadable() {
        let base = serde_json::to_value(recorded(graded_held())).unwrap();
        type FileEdit = fn(&mut serde_json::Value);
        let rows: Vec<(&str, FileEdit)> = vec![
            ("version 2", |v| v["version"] = 2.into()),
            ("a question naming no topic", |v| {
                v["result"] = serde_json::Value::Null;
                v["outcome"] = serde_json::Value::Null;
                v["remediated"] = serde_json::json!({});
                v["exam"]["questions"][0]["topic_id"] = "z".into()
            }),
            ("one answer too few", |v| {
                v["answers"].as_array_mut().unwrap().pop();
            }),
            ("one lock too many", |v| {
                v["locked"].as_array_mut().unwrap().push(false.into())
            }),
            ("one question unit list too few", |v| {
                v["questions_units"].as_array_mut().unwrap().pop();
            }),
            ("one grade too few", |v| {
                v["result"]["grades"].as_array_mut().unwrap().pop();
            }),
            ("one topic too few", |v| {
                v["result"]["topics"].as_array_mut().unwrap().pop();
            }),
            ("a verdict outside the three labels", |v| {
                v["result"]["grades"][0]["verdict"] = "pass".into()
            }),
        ];
        for (row, edit) in rows {
            let mut value = base.clone();
            edit(&mut value);
            let error = load_text(&value.to_string()).expect_err(row);
            assert!(
                format!("{error:#}").contains("unreadable held exam"),
                "row {row}: error was: {error:#}"
            );
        }
        load_text(&base.to_string()).expect("control: the unedited file");
    }

    #[test]
    fn an_unknown_field_inside_any_held_struct_makes_the_held_file_unreadable() {
        let mut held = recorded(graded_held());
        if let Some(result) = held.result.as_mut() {
            result.grades[0].missed_units = vec![rendered_of_every_type()];
        }
        let base = serde_json::to_value(&held).unwrap();
        load_text(&base.to_string()).expect("control: the unedited file");
        for place in [
            "",
            "/start",
            "/result",
            "/result/verdicts",
            "/result/grades/0",
            "/result/topics/0",
            "/outcome",
            "/outcome/standing",
            "/questions_units/0/0",
            "/questions_units/0/0/runs/0",
            "/result/grades/0/feedback_units/0",
            "/result/grades/0/missed_units/0/0/items/0",
            "/result/grades/0/missed_units/0/1/regions/0",
            "/result/grades/0/missed_units/0/2/runs/0/math",
        ] {
            let mut value = base.clone();
            value.pointer_mut(place).unwrap()["extra"] = true.into();
            let error = load_text(&value.to_string())
                .expect_err(&format!("an unknown field at {place:?} was accepted"));
            assert!(
                format!("{error:#}").contains("unreadable held exam"),
                "unknown field at {place:?}: error was: {error:#}"
            );
        }
    }

    fn rendered_of_every_type() -> Vec<ContentUnit> {
        let run = crate::inline::InlineRun {
            text: "t".into(),
            ..Default::default()
        };
        vec![
            ContentUnit::Checklist {
                items: vec![crate::render::ChecklistItem {
                    checked: false,
                    text: "t".into(),
                    runs: vec![run.clone()],
                }],
            },
            ContentUnit::Diagram {
                src: "d.png".into(),
                width: 1,
                height: 1,
                alt: "d".into(),
                regions: vec![crate::review::RegionView {
                    role: crate::review::RegionRole::Asked,
                    reveal_on_answer: true,
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                    unit: "px".into(),
                }],
                revealed_alt: None,
            },
            ContentUnit::Sentence {
                text: "t".into(),
                runs: vec![crate::inline::InlineRun {
                    math: Some(crate::math::MathView {
                        display: false,
                        svg: None,
                        error: None,
                    }),
                    ..run
                }],
            },
        ]
    }

    #[test]
    fn grade_request_refuses_answers_over_the_limit_and_sends_answers_at_it() {
        let mut over = held();
        over.set_answers(
            strings(&[&"x".repeat(MAX_ANSWERS_BYTES), "y"]),
            vec![false, false],
        )
        .unwrap();
        let error = over.grade_request().unwrap_err();
        assert_eq!(error.to_string(), ANSWER_TOO_LONG, "one byte over");
        let mut at = held();
        at.set_answers(
            strings(&[&"x".repeat(MAX_ANSWERS_BYTES - 1), "y"]),
            vec![false, false],
        )
        .unwrap();
        let request = at.grade_request().expect("at the limit");
        GradeRequest::parse(request.to_json().unwrap().as_bytes())
            .expect("the desktop reads the request at the limit");
    }

    #[test]
    fn an_outcome_or_remediation_no_flow_produces_is_neither_loaded_nor_saved() {
        let mut legal = recorded(graded_held());
        legal.remediated.insert("a".into(), 1);
        type HeldEdit = fn(&mut HeldExam);
        let rows: Vec<(&str, HeldEdit)> = vec![
            ("an outcome without a result", |h| {
                h.result = None;
                h.remediated.clear();
            }),
            (
                "a recorded outcome over a result not chosen to be recorded",
                |h| {
                    if let Some(result) = h.result.as_mut() {
                        result.recorded = false;
                    }
                },
            ),
            ("remediation without a recorded outcome", |h| {
                h.outcome = Some(HeldOutcome {
                    recorded: false,
                    standing: None,
                })
            }),
        ];
        let dir = tempfile::tempdir().unwrap();
        for (row, edit) in rows {
            let mut value = legal.clone();
            edit(&mut value);
            let path = dir.path().join(format!("{row}.json"));
            let error = value.save(&path).expect_err(row);
            assert!(!path.exists(), "row {row}: save wrote a file ({error:#})");
            let error = load_text(&serde_json::to_string(&value).unwrap()).expect_err(row);
            assert!(
                format!("{error:#}").contains("unreadable held exam"),
                "row {row}: load error was: {error:#}"
            );
        }
        let path = dir.path().join("legal.json");
        legal.save(&path).expect("control: save");
        assert_eq!(
            HeldExam::load(&path).expect("control: load"),
            Some(legal),
            "control: round trip"
        );
    }

    #[test]
    fn received_refuses_a_failing_exam_and_an_unknown_key() {
        let carried = exam(&["a"], &["a"]);
        let units = carried.questions_units();
        let mut failing = carried.clone();
        failing.version = 2;
        let mut extended: serde_json::Value =
            serde_json::from_str(&received_text(&carried, &units)).unwrap();
        extended["extra"] = true.into();
        for (row, text) in [
            ("a failing nested exam", received_text(&failing, &units)),
            ("an unknown key", extended.to_string()),
        ] {
            let error = HeldExam::received(&text, start()).expect_err(row);
            assert!(
                format!("{error:#}").contains("unreadable carried exam"),
                "row {row}: error was: {error:#}"
            );
        }
        HeldExam::received(&received_text(&carried, &units), start()).expect("control");
    }

    #[test]
    fn set_answers_refuses_a_graded_exam_and_lists_of_the_wrong_length() {
        let graded = graded_held();
        let fresh = held();
        let rows = [
            (
                "a graded exam",
                &graded,
                strings(&["a", "b"]),
                vec![true, true],
            ),
            (
                "one answer too few",
                &fresh,
                strings(&["a"]),
                vec![true, true],
            ),
            (
                "one lock too many",
                &fresh,
                strings(&["a", "b"]),
                vec![true, true, true],
            ),
        ];
        for (row, value, answers, locked) in rows {
            let mut changed = value.clone();
            assert!(
                changed.set_answers(answers, locked).is_err(),
                "row {row}: accepted"
            );
            assert_eq!(&changed, value, "row {row}: value after the refusal");
        }
    }

    #[test]
    fn topic_id_of_maps_request_numbers_to_asked_topics_and_nothing_past_the_end() {
        let value = held();
        assert_eq!(value.topic_id_of(0), Some("a"), "number 0");
        assert_eq!(value.topic_id_of(1), Some("c"), "number 1");
        assert_eq!(value.topic_id_of(2), None, "past the end");
    }

    #[test]
    fn a_locked_whitespace_only_answer_counts_nothing_toward_the_limit() {
        let mut value = held();
        value
            .set_answers(
                strings(&[&"x".repeat(MAX_ANSWERS_BYTES), " \n "]),
                vec![true, true],
            )
            .expect("the blank answer counts 0");
    }

    #[test]
    fn an_embedded_ideas_document_of_another_version_is_refused_at_every_entry() {
        let mut carried = exam(&["a"], &["a"]);
        carried.ideas.version = 2;
        let units = carried.questions_units();
        let mut held_file = serde_json::to_value(held()).unwrap();
        held_file["exam"]["ideas"]["version"] = 2.into();
        let rows: [(&str, Result<()>); 3] = [
            (
                "CarriedExam::parse",
                CarriedExam::parse(&serde_json::to_string(&carried).unwrap()).map(drop),
            ),
            (
                "HeldExam::received",
                HeldExam::received(&received_text(&carried, &units), start()).map(drop),
            ),
            (
                "HeldExam::load",
                load_text(&held_file.to_string()).map(drop),
            ),
        ];
        let mut accepted = Vec::new();
        for (row, outcome) in rows {
            match outcome {
                Ok(()) => accepted.push(row),
                Err(error) => assert!(
                    format!("{error:#}").contains("ideas"),
                    "{row}: error was: {error:#}"
                ),
            }
        }
        assert!(
            accepted.is_empty(),
            "accepted ideas version 2: {accepted:?}"
        );
    }

    #[test]
    fn a_grading_reply_with_an_unknown_top_level_field_still_parses() {
        let reply = fitting_reply(&held());
        let mut value = serde_json::to_value(&reply).unwrap();
        value["added_later"] = true.into();
        let parsed: GradingReply = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, reply);
    }

    #[test]
    fn a_grading_reply_with_an_unknown_field_inside_topic_number_cards_is_refused() {
        let mut value = serde_json::to_value(fitting_reply(&held())).unwrap();
        value["cards_by_topic"][0]["extra"] = true.into();
        assert!(serde_json::from_value::<GradingReply>(value).is_err());
    }
}
