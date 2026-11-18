use std::{collections::HashSet, path::Path};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    config::Strictness,
    exam_ideas::IdeasDocument,
    render::{ContentUnit, exam_text_units},
};

pub const VERSION: u32 = 1;
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
            version: VERSION,
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
        if self.version != VERSION {
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
}
