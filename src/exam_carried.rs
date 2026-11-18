use std::{collections::HashSet, path::Path};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    config::Strictness,
    exam_ideas::IdeasDocument,
    render::{ContentUnit, exam_text_units},
};

pub const VERSION: u32 = 1;

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
        let text = with_field(&exam(&["a"], &[]), |v| v["extra"] = true.into());
        assert_unreadable(&text);
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
}
