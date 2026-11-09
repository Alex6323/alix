use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const QUESTIONS_PER_TOPIC: usize = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdeasDocument {
    pub version: u32,
    pub material: String,
    pub topics: Vec<IdeaTopic>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdeaTopic {
    pub heading: String,
    pub ideas: Vec<Idea>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Idea {
    pub text: String,
    pub last_asked_ms: Option<u64>,
    pub last_passed: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskedIdea {
    pub heading: String,
    pub idea: usize,
    pub passed: bool,
}

impl IdeasDocument {
    pub fn new(material: String) -> Self {
        Self {
            version: VERSION,
            material,
            topics: Vec::new(),
        }
    }

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

    pub fn topic(&self, heading: &str) -> Option<&IdeaTopic> {
        self.topics.iter().find(|topic| topic.heading == heading)
    }

    /// The document to examine with: this one when it was listed from the same
    /// material, otherwise an empty one for that material.
    pub fn for_material(doc: Option<Self>, material: &str) -> Self {
        doc.filter(|doc| doc.material == material)
            .unwrap_or_else(|| Self::new(material.to_string()))
    }

    pub fn missing_headings<'a>(&self, headings: &[&'a str]) -> Vec<&'a str> {
        headings
            .iter()
            .copied()
            .filter(|heading| self.topic(heading).is_none_or(|s| s.ideas.is_empty()))
            .collect()
    }

    pub fn set_topic(&mut self, heading: &str, ideas: Vec<String>) {
        let ideas = ideas
            .into_iter()
            .map(|text| Idea {
                text,
                ..Idea::default()
            })
            .collect();
        match self.topics.iter_mut().find(|s| s.heading == heading) {
            Some(topic) => topic.ideas = ideas,
            None => self.topics.push(IdeaTopic {
                heading: heading.to_string(),
                ideas,
            }),
        }
    }

    /// Up to `QUESTIONS_PER_TOPIC` ideas of a topic, in the order an exam
    /// should ask them: never asked, then last failed, then asked longest ago.
    pub fn select(&self, heading: &str) -> Vec<usize> {
        let Some(topic) = self.topic(heading) else {
            return Vec::new();
        };
        let mut order: Vec<usize> = (0..topic.ideas.len()).collect();
        order.sort_by_key(|&at| {
            let idea = &topic.ideas[at];
            let rank = match (idea.last_asked_ms, idea.last_passed) {
                (None, _) => 0,
                (Some(_), Some(false)) => 1,
                (Some(_), _) => 2,
            };
            (rank, idea.last_asked_ms.unwrap_or(0), at)
        });
        order.truncate(QUESTIONS_PER_TOPIC);
        order
    }

    pub fn record(&mut self, asked: &[AskedIdea], now_ms: u64) {
        for entry in asked {
            if let Some(idea) = self
                .topics
                .iter_mut()
                .find(|s| s.heading == entry.heading)
                .and_then(|s| s.ideas.get_mut(entry.idea))
            {
                idea.last_asked_ms = Some(now_ms);
                idea.last_passed = Some(entry.passed);
            }
        }
    }
}

pub fn material_fingerprint(material: &str) -> String {
    use std::hash::Hasher;
    let mut hasher = twox_hash::XxHash64::default();
    hasher.write(material.as_bytes());
    format!("xxh64-{:016x}", hasher.finish())
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
            material: "xxh64-0".to_string(),
            topics: vec![IdeaTopic {
                heading: "H".to_string(),
                ideas: ideas
                    .iter()
                    .enumerate()
                    .map(|(at, &(last_asked_ms, last_passed))| Idea {
                        text: format!("idea {at}"),
                        last_asked_ms,
                        last_passed,
                    })
                    .collect(),
            }],
        }
    }

    #[test]
    fn selection_takes_never_asked_then_failed_then_oldest_three() {
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
            assert_eq!(expected.to_vec(), doc(ideas).select("H"), "{label}");
        }
        assert!(doc(&[]).select("missing").is_empty(), "unknown heading");
    }

    #[test]
    fn recording_marks_only_the_asked_ideas() {
        let mut d = doc(&[(None, None); 3]);
        d.record(
            &[
                AskedIdea {
                    heading: "H".to_string(),
                    idea: 1,
                    passed: false,
                },
                AskedIdea {
                    heading: "gone".to_string(),
                    idea: 0,
                    passed: true,
                },
                AskedIdea {
                    heading: "H".to_string(),
                    idea: 7,
                    passed: true,
                },
            ],
            42,
        );
        let marked: Vec<_> = d.topics[0]
            .ideas
            .iter()
            .map(|i| (i.last_asked_ms, i.last_passed))
            .collect();
        assert_eq!(
            vec![(None, None), (Some(42), Some(false)), (None, None)],
            marked
        );
    }

    #[test]
    fn a_new_material_starts_an_empty_document_and_the_same_keeps_it() {
        let kept = doc(&[(Some(1), Some(true))]);
        assert_eq!(
            kept,
            IdeasDocument::for_material(Some(kept.clone()), "xxh64-0"),
            "same material keeps history"
        );
        let fresh = IdeasDocument::for_material(Some(kept), "xxh64-1");
        assert!(fresh.topics.is_empty(), "changed material drops history");
        assert_eq!("xxh64-1", fresh.material);
    }

    #[test]
    fn missing_headings_are_those_without_ideas() {
        let mut d = doc(&[(None, None)]);
        d.set_topic("Empty", Vec::new());
        assert_eq!(
            vec!["New", "Empty"],
            d.missing_headings(&["H", "New", "Empty"])
        );
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
            r#"{"version":1,"material":"m","topics":[],"extra":1}"#,
            r#"{"version":2,"material":"m","topics":[]}"#,
            r#"{"version":1,"material":"m","topics":[{"heading":"H","ideas":[{"text":"t","last_asked_ms":null,"last_passed":null,"model":"x"}]}]}"#,
        ];
        for text in rejected {
            assert!(IdeasDocument::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn the_fingerprint_names_its_hash_and_follows_the_bytes() {
        let a = material_fingerprint("a");
        assert!(a.starts_with("xxh64-") && a.len() == 22, "{a}");
        assert_eq!(a, material_fingerprint("a"));
        assert_ne!(a, material_fingerprint("b"));
    }
}
