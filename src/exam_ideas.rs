use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const QUESTIONS_PER_TOPIC: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdeasDocument {
    pub version: u32,
    pub topics: BTreeMap<String, IdeaTopic>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdeaTopic {
    pub fingerprint: String,
    pub ideas: Vec<Idea>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Idea {
    pub text: String,
    pub last_tested_ms: Option<u64>,
    pub last_test_passed: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskedIdea {
    pub topic: String,
    pub idea: usize,
    pub passed: bool,
}

impl Default for IdeasDocument {
    fn default() -> Self {
        Self {
            version: VERSION,
            topics: BTreeMap::new(),
        }
    }
}

impl IdeasDocument {
    pub fn parse(text: &str) -> Result<Self> {
        let doc: Self = serde_json::from_str(text).context("unreadable exam ideas document")?;
        if doc.version != VERSION {
            bail!("unreadable exam ideas document");
        }
        Ok(doc)
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("cannot encode the exam ideas document")
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
        write_atomic(path, self.to_json()?.as_bytes())
    }

    /// The topic's entry for this fingerprint: a stale entry is replaced by an
    /// empty one, so its ideas are listed again.
    pub fn for_topic(&mut self, topic_id: &str, fingerprint: &str) -> &mut IdeaTopic {
        let entry = self.topics.entry(topic_id.to_string()).or_default();
        if entry.fingerprint != fingerprint {
            *entry = IdeaTopic {
                fingerprint: fingerprint.to_string(),
                ideas: Vec::new(),
            };
        }
        entry
    }

    pub fn select(&self, topic_id: &str) -> Vec<usize> {
        self.topics
            .get(topic_id)
            .map(IdeaTopic::select)
            .unwrap_or_default()
    }

    pub fn record(&mut self, asked: &[AskedIdea], now_ms: u64) {
        for entry in asked {
            if let Some(idea) = self
                .topics
                .get_mut(&entry.topic)
                .and_then(|topic| topic.ideas.get_mut(entry.idea))
            {
                idea.last_tested_ms = Some(now_ms);
                idea.last_test_passed = Some(entry.passed);
            }
        }
    }
}

impl IdeaTopic {
    pub fn select(&self) -> Vec<usize> {
        select_ideas(&self.ideas)
    }
}

/// Up to `QUESTIONS_PER_TOPIC` ideas, in the order an exam should ask them:
/// never tested, then last failed, then tested longest ago.
pub fn select_ideas(ideas: &[Idea]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..ideas.len()).collect();
    order.sort_by_key(|&at| {
        let idea = &ideas[at];
        let rank = match (idea.last_tested_ms, idea.last_test_passed) {
            (None, _) => 0,
            (Some(_), Some(false)) => 1,
            (Some(_), _) => 2,
        };
        (rank, idea.last_tested_ms.unwrap_or(0), at)
    });
    order.truncate(QUESTIONS_PER_TOPIC);
    order
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        crate::fsio::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    crate::fsio::replace_file(&tmp, path, bytes)
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    type Asked = (Option<u64>, Option<bool>);

    fn doc(ideas: &[Asked]) -> IdeasDocument {
        IdeasDocument {
            version: VERSION,
            topics: BTreeMap::from([(
                "topic-h".to_string(),
                IdeaTopic {
                    fingerprint: "f".to_string(),
                    ideas: ideas
                        .iter()
                        .enumerate()
                        .map(|(at, &(last_tested_ms, last_test_passed))| Idea {
                            text: format!("idea {at}"),
                            last_tested_ms,
                            last_test_passed,
                        })
                        .collect(),
                },
            )]),
        }
    }

    #[test]
    fn selection_takes_never_tested_then_failed_then_oldest_three() {
        let rows: &[(&str, &[Asked], &[usize])] = &[
            ("all fresh, list order", &[(None, None); 5], &[0, 1, 2]),
            (
                "fresh before failed before passed",
                &[
                    (Some(5), Some(true)),
                    (Some(9), Some(false)),
                    (None, None),
                    (Some(1), Some(true)),
                ],
                &[2, 1, 3],
            ),
            (
                "older failure first",
                &[
                    (Some(9), Some(false)),
                    (Some(2), Some(false)),
                    (Some(1), Some(true)),
                ],
                &[1, 0, 2],
            ),
            ("fewer ideas than three", &[(None, None); 2], &[0, 1]),
        ];
        for (label, ideas, expected) in rows {
            assert_eq!(expected.to_vec(), doc(ideas).select("topic-h"), "{label}");
        }
        assert!(doc(&[]).select("topic-missing").is_empty(), "unknown topic");
    }

    #[test]
    fn recording_marks_only_the_tested_ideas() {
        let mut d = doc(&[(None, None); 3]);
        let asked = |topic: &str, idea, passed| AskedIdea {
            topic: topic.to_string(),
            idea,
            passed,
        };
        d.record(
            &[
                asked("topic-h", 1, false),
                asked("topic-gone", 0, true),
                asked("topic-h", 7, true),
            ],
            42,
        );
        let marked: Vec<_> = d.topics["topic-h"]
            .ideas
            .iter()
            .map(|i| (i.last_tested_ms, i.last_test_passed))
            .collect();
        assert_eq!(
            vec![(None, None), (Some(42), Some(false)), (None, None)],
            marked
        );
    }

    #[test]
    fn a_topic_keeps_its_ideas_for_its_fingerprint_and_drops_them_for_another() {
        let mut d = doc(&[(Some(1), Some(true))]);
        let kept = d.topics["topic-h"].clone();
        assert_eq!(&kept, d.for_topic("topic-h", "f"), "same fingerprint");
        let other = d.for_topic("topic-h", "g");
        assert!(other.ideas.is_empty(), "changed fingerprint drops history");
        assert_eq!("g", other.fingerprint);
        let fresh = d.for_topic("topic-new", "n");
        assert!(
            fresh.ideas.is_empty() && fresh.fingerprint == "n",
            "new topic"
        );
        assert_eq!(2, d.topics.len(), "one entry per topic id");
    }

    #[test]
    fn the_document_round_trips_and_rejects_unknown_fields_and_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".alix/exam/deck-x.json");
        assert_eq!(None, IdeasDocument::load(&path).unwrap(), "absent is none");
        let d = doc(&[(Some(3), Some(false))]);
        d.save(&path).unwrap();
        assert_eq!(Some(d), IdeasDocument::load(&path).unwrap());
        let rejected = [
            r#"{"version":1,"topics":{},"extra":1}"#,
            r#"{"version":2,"topics":{}}"#,
            r#"{"version":1,"material":"m","topics":{}}"#,
            r#"{"version":1,"topics":{"topic-h":{"fingerprint":"f","heading":"H","ideas":[]}}}"#,
            r#"{"version":1,"topics":{"topic-h":{"fingerprint":"f","ideas":[{"text":"t","last_asked_ms":null,"last_passed":null}]}}}"#,
        ];
        for text in rejected {
            assert!(IdeasDocument::parse(text).is_err(), "{text}");
        }
    }
}
