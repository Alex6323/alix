use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExamTopicOutcome {
    pub topic_id: String,
    pub fingerprint: String,
    pub pass: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExamStamp {
    pub sourced: String,
    pub unsourced_cards: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExamOutcome {
    pub topics: Vec<ExamTopicOutcome>,
    pub full: bool,
    pub pass: u32,
    pub partial: u32,
    pub total: u32,
    pub eligible: bool,
    pub stamp: ExamStamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExamStart {
    pub cards: Option<Vec<String>>,
    pub eligible: bool,
    pub stamp: ExamStamp,
    pub unsourced_topics: Vec<String>,
}

fn open_store(
    deck_path: &Path,
    root_dir: &str,
    device: Option<String>,
) -> Result<alix::store::Store> {
    let root_store = alix::workspace::root_store_path(Path::new(root_dir));
    let mut store = alix::assemble::store_for(&[deck_path.to_path_buf()], Some(&root_store))?;
    if device.is_some() {
        store.device = device;
    }
    Ok(store)
}

fn exam_ideas_path(deck_path: &Path, root_dir: &Path) -> Result<PathBuf> {
    let (files, deck_id) = exam_files(deck_path, root_dir)?;
    Ok(files.exam_ideas_for(&deck_id))
}

fn exam_answers_path(deck_path: &Path, root_dir: &Path) -> Result<PathBuf> {
    let (files, deck_id) = exam_files(deck_path, root_dir)?;
    Ok(files.exam_prepared_for(&deck_id))
}

fn exam_files(deck_path: &Path, root_dir: &Path) -> Result<(alix::state::UserFiles, String)> {
    let deck = alix::deck::Deck::load(deck_path)?;
    let deck_id = deck.deck_token.context("the deck is not initialized")?;
    let root_store = alix::workspace::root_store_path(root_dir);
    Ok((alix::state::UserFiles::new(root_store), deck_id))
}

#[flutter_rust_bridge::frb(sync)]
pub fn exam_ideas_load(deck_path: String, root_dir: String) -> Result<Option<String>> {
    let path = exam_ideas_path(Path::new(&deck_path), Path::new(&root_dir))?;
    alix::exam_ideas::IdeasDocument::load(&path)?
        .map(|document| document.to_json())
        .transpose()
}

#[flutter_rust_bridge::frb(sync)]
pub fn exam_ideas_save(deck_path: String, root_dir: String, json: String) -> Result<()> {
    let path = exam_ideas_path(Path::new(&deck_path), Path::new(&root_dir))?;
    alix::exam_ideas::IdeasDocument::parse(&json)?.save(&path)
}

const PHONE_PREPARED_VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PhonePreparedExam {
    version: u32,
    questions: Vec<String>,
    answers: Vec<String>,
}

impl PhonePreparedExam {
    fn parse(text: &str) -> Result<Self> {
        let document: Self =
            serde_json::from_str(text).context("unreadable phone exam document")?;
        if document.version != PHONE_PREPARED_VERSION
            || document.questions.len() != document.answers.len()
        {
            bail!("unreadable phone exam document");
        }
        Ok(document)
    }

    fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("the phone exam path has no parent")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
        let temporary = path.with_extension("json.tmp");
        let json =
            serde_json::to_vec_pretty(self).context("cannot encode the phone exam document")?;
        std::fs::write(&temporary, json)
            .with_context(|| format!("cannot write {}", temporary.display()))?;
        std::fs::rename(&temporary, path)
            .with_context(|| format!("cannot replace {}", path.display()))
    }
}

#[flutter_rust_bridge::frb(sync)]
pub fn exam_answers_load(
    deck_path: String,
    root_dir: String,
    questions: Vec<String>,
) -> Result<Vec<String>> {
    let path = exam_answers_path(Path::new(&deck_path), Path::new(&root_dir))?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(vec![String::new(); questions.len()]);
        }
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    let document =
        PhonePreparedExam::parse(&text).with_context(|| format!("reading {}", path.display()))?;
    if document.questions != questions {
        std::fs::remove_file(&path)
            .with_context(|| format!("cannot remove stale {}", path.display()))?;
        return Ok(vec![String::new(); questions.len()]);
    }
    Ok(document.answers)
}

#[flutter_rust_bridge::frb(sync)]
pub fn exam_answers_save(
    deck_path: String,
    root_dir: String,
    questions: Vec<String>,
    answers: Vec<String>,
) -> Result<()> {
    if questions.len() != answers.len() {
        bail!("one phone exam answer is required per question");
    }
    let path = exam_answers_path(Path::new(&deck_path), Path::new(&root_dir))?;
    PhonePreparedExam {
        version: PHONE_PREPARED_VERSION,
        questions,
        answers,
    }
    .save(&path)
}

#[flutter_rust_bridge::frb(sync)]
pub fn exam_answers_clear(deck_path: String, root_dir: String) -> Result<()> {
    let path = exam_answers_path(Path::new(&deck_path), Path::new(&root_dir))?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("cannot remove {}", path.display())),
    }
}

/// `cards` is `None` for a full sitting. The review condition and the
/// stamp are evaluated here, once, against the phone's own store.
#[flutter_rust_bridge::frb(sync)]
pub fn exam_sitting_start(
    deck_path: String,
    root_dir: String,
    needs_work_only: bool,
    now_ms: u64,
) -> Result<ExamStart> {
    let path = PathBuf::from(deck_path);
    alix::assemble::stamp_for_session(&path)?;
    let deck = alix::deck::Deck::load(&path)?;
    let store = open_store(&path, &root_dir, None)?;
    let cards = needs_work_only.then(|| alix::exam_status::needs_work_card_ids(&deck, &store));
    let retire_after_days =
        alix::exam_status::retire_after_days_for(alix::config::ReviewConfig::default(), &path);
    let start = alix::exam_status::sitting_start(&deck, &store, now_ms, retire_after_days);
    Ok(ExamStart {
        cards,
        eligible: start.eligible,
        stamp: ExamStamp {
            sourced: start.stamp.sourced,
            unsourced_cards: start.stamp.unsourced_cards,
        },
        unsourced_topics: start.unsourced_topics,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExamApplied {
    pub unlocked: bool,
    pub exam_high_mark: Option<String>,
    pub exam_changed: bool,
}

#[flutter_rust_bridge::frb(sync)]
pub fn apply_exam_result(
    deck_path: String,
    root_dir: String,
    outcome: ExamOutcome,
    now_ms: u64,
    device: Option<String>,
) -> Result<ExamApplied> {
    let path = PathBuf::from(deck_path);
    let deck = alix::deck::Deck::load(&path)?;
    let mut store = open_store(&path, &root_dir, device)?;
    let record = alix::store::SittingRecord {
        topics: outcome
            .topics
            .into_iter()
            .map(|topic| alix::store::TopicRecord {
                topic_id: topic.topic_id,
                fingerprint: topic.fingerprint,
                pass: topic.pass,
            })
            .collect(),
        full: outcome.full,
        pass: outcome.pass,
        partial: outcome.partial,
        total: outcome.total,
    };
    let stamp = alix::store::ExamStamp {
        sourced: outcome.stamp.sourced,
        unsourced_cards: outcome.stamp.unsourced_cards,
    };
    let unlocked = alix::exam_status::record_sitting_checked(
        &mut store,
        &deck,
        &alix::exam_status::examined_topics(&deck),
        &record,
        now_ms,
        outcome.eligible,
        &stamp,
    )?;
    store.save()?;
    let deck_id = deck.deck_token.as_deref().unwrap_or_default();
    Ok(ExamApplied {
        unlocked,
        exam_high_mark: alix::exam_status::exam_high_mark(&store, deck_id, now_ms),
        exam_changed: alix::exam_status::standing_changed(&store, deck_id, &stamp),
    })
}

#[flutter_rust_bridge::frb(sync)]
pub fn apply_exam_remediation(
    deck_path: String,
    root_dir: String,
    topic_id: String,
    cards_text: String,
    device: Option<String>,
) -> Result<u32> {
    let path = PathBuf::from(deck_path);
    let deck = alix::deck::Deck::load(&path)?;
    let fingerprints = deck
        .cards
        .iter()
        .map(|card| card.block_fingerprint)
        .collect();
    let deck_token = deck.deck_token.unwrap_or_default();
    let mut store = open_store(&path, &root_dir, device)?;
    let count = alix::store::store_remediation_cards(
        &mut store,
        Some(&path),
        &deck_token,
        &topic_id,
        &fingerprints,
        &cards_text,
        None,
    )
    .into_result()?;
    Ok(count as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000_000;

    fn two_topic_deck(root: &Path) -> PathBuf {
        crate::api::testkit::sourced_member(
            root,
            "d.md",
            "---\nformat-version: 1\nid: deck-d\n---\n\
             # A\nprose\n<!-- id: topic-a -->\n\n## a1\nx\n<!-- id: card-a1 -->\n\n\
             # B\nprose\n<!-- id: topic-b -->\n\n## b1\nx\n<!-- id: card-b1 -->\n",
        )
    }

    fn reopened(path: &Path, root: &Path) -> alix::store::Store {
        open_store(path, &root.to_string_lossy(), None).unwrap()
    }

    #[test]
    fn exam_ideas_round_trip_in_the_phone_store() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let deck_path = path.to_string_lossy().into_owned();
        let root_dir = root.to_string_lossy().into_owned();

        assert_eq!(
            None,
            exam_ideas_load(deck_path.clone(), root_dir.clone()).unwrap()
        );

        let first = r#"{"version":1,"topics":{}}"#;
        exam_ideas_save(deck_path.clone(), root_dir.clone(), first.to_string()).unwrap();
        assert_eq!(
            Some(
                alix::exam_ideas::IdeasDocument::parse(first)
                    .unwrap()
                    .to_json()
                    .unwrap()
            ),
            exam_ideas_load(deck_path.clone(), root_dir.clone()).unwrap()
        );
        assert!(root.join(".alix/exam/deck-d.json").is_file());
        assert!(!root.join(".alix/exam/deck-d.json.tmp").exists());

        let second = r#"{"version":1,"topics":{"topic-a":{"fingerprint":"f","ideas":[]}}}"#;
        exam_ideas_save(deck_path.clone(), root_dir.clone(), second.to_string()).unwrap();
        assert_eq!(
            Some(
                alix::exam_ideas::IdeasDocument::parse(second)
                    .unwrap()
                    .to_json()
                    .unwrap()
            ),
            exam_ideas_load(deck_path.clone(), root_dir.clone()).unwrap()
        );
        assert!(
            exam_ideas_save(
                deck_path,
                root_dir,
                r#"{"version":1,"extra":true}"#.to_string()
            )
            .is_err()
        );
    }

    #[test]
    fn phone_answers_resume_only_the_exact_question_set() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let deck_path = path.to_string_lossy().into_owned();
        let root_dir = root.to_string_lossy().into_owned();
        let questions = vec!["one?".to_string(), "two?".to_string()];

        assert_eq!(
            vec!["", ""],
            exam_answers_load(deck_path.clone(), root_dir.clone(), questions.clone()).unwrap()
        );
        exam_answers_save(
            deck_path.clone(),
            root_dir.clone(),
            questions.clone(),
            vec!["first".to_string(), String::new()],
        )
        .unwrap();
        assert_eq!(
            vec!["first", ""],
            exam_answers_load(deck_path.clone(), root_dir.clone(), questions).unwrap()
        );
        let prepared = root.join(".alix/exam/deck-d.prepared.json");
        assert!(prepared.is_file());

        assert_eq!(
            vec![""],
            exam_answers_load(
                deck_path.clone(),
                root_dir.clone(),
                vec!["replacement?".to_string()]
            )
            .unwrap()
        );
        assert!(
            !prepared.exists(),
            "a changed question list deletes stale answers"
        );

        exam_answers_save(
            deck_path.clone(),
            root_dir.clone(),
            vec!["replacement?".to_string()],
            vec!["answer".to_string()],
        )
        .unwrap();
        exam_answers_clear(deck_path, root_dir).unwrap();
        assert!(!prepared.exists());
    }

    #[test]
    fn phone_answers_reject_an_unknown_field() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let prepared = root.join(".alix/exam/deck-d.prepared.json");
        std::fs::create_dir_all(prepared.parent().unwrap()).unwrap();
        std::fs::write(
            prepared,
            r#"{"version":1,"questions":[],"answers":[],"unknown":true}"#,
        )
        .unwrap();

        assert!(
            exam_answers_load(
                path.to_string_lossy().into_owned(),
                root.to_string_lossy().into_owned(),
                Vec::new()
            )
            .is_err()
        );
    }

    fn root_arg(root: &Path) -> String {
        root.to_string_lossy().into_owned()
    }

    fn topic_outcome(path: &Path, heading: &str, pass: bool) -> ExamTopicOutcome {
        let deck = alix::deck::Deck::load(path).unwrap();
        let topic = alix::exam_status::examined_topics(&deck)
            .into_iter()
            .find(|topic| topic.label == heading)
            .expect("a sourced topic with that heading");
        ExamTopicOutcome {
            topic_id: topic.topic_id,
            fingerprint: topic.fingerprint,
            pass,
        }
    }

    fn outcome(start: &ExamStart, topics: Vec<ExamTopicOutcome>, full: bool) -> ExamOutcome {
        let total = topics.len() as u32;
        let pass = topics.iter().filter(|topic| topic.pass).count() as u32;
        ExamOutcome {
            topics,
            full,
            pass,
            partial: 0,
            total,
            eligible: start.eligible,
            stamp: start.stamp.clone(),
        }
    }

    #[test]
    fn apply_exam_result_records_like_the_desktop() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let deck_path = path.to_string_lossy().into_owned();

        let start =
            exam_sitting_start(deck_path.clone(), root_arg(root), false, NOW).unwrap();
        assert_eq!(None, start.cards, "a full sitting sends no card list");
        assert!(start.eligible, "no unsourced card: the condition holds");
        let applied = apply_exam_result(
            deck_path.clone(),
            root_arg(root),
            outcome(
                &start,
                vec![
                    topic_outcome(&path, "A", true),
                    topic_outcome(&path, "B", false),
                ],
                true,
            ),
            NOW,
            None,
        )
        .unwrap();
        let store = reopened(&path, root);
        assert!(
            !applied.unlocked,
            "full sitting with a failed topic: no unlock"
        );
        assert_eq!(
            None, applied.exam_high_mark,
            "neither passed nor mastered: no standing"
        );
        assert_eq!(
            Some(alix::store::FullExamResult {
                at_ms: NOW,
                pass: 1,
                partial: 0,
                total: 2,
            }),
            store.full_exam_result("deck-d"),
            "a full sitting keeps its count"
        );
        assert_eq!(
            Some(false),
            store.topic_result("deck-d", "topic-b").map(|r| r.pass),
            "each examined topic carries its verdict by topic id"
        );
        assert_eq!(None, store.exam_stamp("deck-d"), "a failed topic stamps nothing");
        let filtered =
            exam_sitting_start(deck_path.clone(), root_arg(root), true, NOW + 1).unwrap();
        assert_eq!(
            Some(vec!["card-b1".to_string()]),
            filtered.cards,
            "the filter names only the topic that needs work"
        );

        let applied = apply_exam_result(
            deck_path.clone(),
            root_arg(root),
            outcome(&filtered, vec![topic_outcome(&path, "B", true)], false),
            NOW + 1,
            None,
        )
        .unwrap();
        let store = reopened(&path, root);
        assert!(
            applied.unlocked,
            "a filtered sitting passing the last topic unlocks"
        );
        assert!(
            applied
                .exam_high_mark
                .as_deref()
                .is_some_and(|line| line.starts_with("passed (")),
            "the phone words its own standing: {:?}",
            applied.exam_high_mark
        );
        assert_eq!(Some(NOW + 1), store.deck_passed_at("deck-d"));
        assert_eq!(
            Some(&alix::store::ExamStamp {
                sourced: filtered.stamp.sourced.clone(),
                unsourced_cards: Vec::new(),
            }),
            store.exam_stamp("deck-d"),
            "the stamp captured at the phone's start is written with the pass"
        );
        assert_eq!(
            Some(NOW),
            store.full_exam_result("deck-d").map(|r| r.at_ms),
            "a filtered sitting leaves the full count alone"
        );
        assert_eq!(
            None,
            store.deck_mastered_at("deck-d"),
            "mastered needs a full 100% sitting"
        );
    }

    #[test]
    fn a_phone_result_over_sources_changed_after_its_start_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let deck_path = path.to_string_lossy().into_owned();
        let start = exam_sitting_start(deck_path.clone(), root_arg(root), false, NOW).unwrap();
        let passed = outcome(
            &start,
            vec![
                topic_outcome(&path, "A", true),
                topic_outcome(&path, "B", true),
            ],
            true,
        );

        let live = root.join("live/d.md.txt");
        std::fs::write(&live, "the changed source of d.md\n").unwrap();
        alix::sources::commands::update(
            &path,
            &alix::sources::source_id(b"the source of d.md\n"),
            &live,
            None,
            false,
            &alix::config::ExamConfig::default(),
        )
        .unwrap();

        let error = apply_exam_result(deck_path, root_arg(root), passed, NOW, None).unwrap_err();
        assert_eq!(
            alix::exam_status::MATERIAL_CHANGED,
            format!("{error:#}"),
            "the phone is told the material changed"
        );
        let store = reopened(&path, root);
        assert!(!store.deck_passed("deck-d"), "no standing latches");
        assert_eq!(None, store.deck_mastered_at("deck-d"), "no mastery latches");
        assert_eq!(None, store.exam_stamp("deck-d"), "no stamp is written");
        assert_eq!(
            None,
            store.topic_result("deck-d", "topic-a"),
            "no topic result is written"
        );
        assert_eq!(
            None,
            store.full_exam_result("deck-d"),
            "no full count is written"
        );
    }

    #[test]
    fn an_ineligible_phone_start_records_results_without_a_standing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}\n# C\n\n## c1\nunreviewed\n")).unwrap();
        let deck_path = path.to_string_lossy().into_owned();

        let start =
            exam_sitting_start(deck_path.clone(), root_arg(root), false, NOW).unwrap();
        assert!(
            !start.eligible,
            "an unreviewed card in an unsourced topic fails the condition"
        );
        assert_eq!(
            vec!["C".to_string()],
            start.unsourced_topics,
            "the start names the topic the exam leaves out"
        );
        assert_eq!(
            1,
            start.stamp.unsourced_cards.len(),
            "the stamp lists the unsourced card: {:?}",
            start.stamp.unsourced_cards
        );
        let applied = apply_exam_result(
            deck_path.clone(),
            root_arg(root),
            outcome(
                &start,
                vec![
                    topic_outcome(&path, "A", true),
                    topic_outcome(&path, "B", true),
                ],
                true,
            ),
            NOW,
            None,
        )
        .unwrap();

        let store = reopened(&path, root);
        assert!(!applied.unlocked, "control: no unlock");
        assert_eq!(None, applied.exam_high_mark, "control: no standing");
        assert_eq!(None, store.deck_passed_at("deck-d"));
        assert_eq!(None, store.deck_mastered_at("deck-d"));
        assert_eq!(None, store.exam_stamp("deck-d"));
        assert_eq!(
            Some(true),
            store.topic_result("deck-d", "topic-a").map(|r| r.pass),
            "the results are still recorded"
        );
    }

    #[test]
    fn an_ineligible_phone_sitting_on_changed_material_reports_exam_changed_and_keeps_the_standing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let deck_path = path.to_string_lossy().into_owned();
        let start = exam_sitting_start(deck_path.clone(), root_arg(root), false, NOW).unwrap();
        let both = |start: &ExamStart| {
            outcome(
                start,
                vec![
                    topic_outcome(&path, "A", true),
                    topic_outcome(&path, "B", true),
                ],
                true,
            )
        };
        let first =
            apply_exam_result(deck_path.clone(), root_arg(root), both(&start), NOW, None).unwrap();
        assert!(!first.exam_changed, "a sitting that just set the standing matches it");

        let mut moved = both(&start);
        moved.eligible = false;
        moved.stamp.unsourced_cards = vec!["card-new".to_string()];
        let second =
            apply_exam_result(deck_path.clone(), root_arg(root), moved, NOW + 1, None).unwrap();
        assert!(second.exam_changed, "the stamp now lists a card the standing does not");
        assert!(second.exam_high_mark.is_some(), "the standing stays latched");
    }

    #[test]
    fn a_retired_due_card_in_an_unsourced_topic_does_not_block_the_phone() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            format!("{text}\n# C\n\n## c1\nretired\n<!-- id: card-c1 -->\n"),
        )
        .unwrap();
        {
            let mut store = open_store(&path, &root_arg(root), None).unwrap();
            *store.get_or_insert("card-c1") = alix::store::CardState {
                recall: Some(alix::store::FsrsState {
                    state: 2,
                    scheduled_days: 365,
                    due_ms: NOW - 1,
                    ..Default::default()
                }),
                history: vec![alix::store::Review {
                    ts_ms: NOW - 2,
                    grade: alix::scheduler::Grade::Pass,
                    depth: alix::depth::Depth::Recall,
                    propagated: false,
                    confidence: None,
                }],
                ..alix::store::CardState::new()
            };
            store.save().unwrap();
        }

        let start =
            exam_sitting_start(path.to_string_lossy().into_owned(), root_arg(root), false, NOW)
                .unwrap();
        assert_eq!(
            vec!["card-c1".to_string()],
            start.stamp.unsourced_cards,
            "the retired card is unsourced material"
        );
        assert!(
            start.eligible,
            "a retired card meets the review condition on the phone as on the desktop, due or not"
        );
    }

    #[test]
    fn exam_start_stamps_new_cards_and_a_full_sitting_names_no_cards() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}\n## b2\nnew\n")).unwrap();

        let start =
            exam_sitting_start(path.to_string_lossy().into_owned(), root_arg(root), false, NOW)
                .unwrap();

        assert_eq!(None, start.cards, "a full sitting sends no card list");
        let deck = alix::deck::Deck::load(&path).unwrap();
        assert!(
            deck.cards.iter().all(|card| card.id().is_some()),
            "exam start stamps the new card"
        );
    }

    #[test]
    fn apply_exam_remediation_creates_personal_cards_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let deck_path = path.to_string_lossy().into_owned();
        let remediation = "## a1\nx\n\n## why b?\nbecause\n".to_string();

        let created = apply_exam_remediation(
            deck_path.clone(),
            root_arg(root),
            "topic-a".to_string(),
            remediation.clone(),
            None,
        )
        .unwrap();
        let again = apply_exam_remediation(
            deck_path,
            root_arg(root),
            "topic-a".to_string(),
            remediation,
            None,
        )
        .unwrap();

        assert_eq!(
            (1, 0),
            (created, again),
            "the deck's own card and a repeat are skipped"
        );
        assert_eq!(1, alix::personal::read(&path, "deck-d").cards.len());
    }

    #[cfg(unix)]
    #[test]
    fn apply_exam_remediation_keeps_a_card_written_before_save_failed() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let deck_path = path.to_string_lossy().into_owned();
        let root_dir = root_arg(root);
        let progress = alix::state::UserFiles::new(root).progress();
        std::fs::create_dir_all(&progress).unwrap();
        std::fs::set_permissions(&progress, std::fs::Permissions::from_mode(0o555)).unwrap();

        let error = apply_exam_remediation(
            deck_path.clone(),
            root_dir.clone(),
            "topic-a".to_string(),
            "## Why b?\nbecause\n".to_string(),
            None,
        )
        .unwrap_err();

        std::fs::set_permissions(&progress, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            error.to_string().contains("progress"),
            "the fixture must fail the progress save: {error:#}"
        );
        let personal = alix::personal::read(&path, "deck-d");
        let id = personal.cards[0]
            .card
            .id()
            .expect("the sidecar card is stamped");
        assert_eq!(
            0,
            apply_exam_remediation(
                deck_path,
                root_dir,
                "topic-a".to_string(),
                "## Why b?\nbecause\n".to_string(),
                None,
            )
            .unwrap(),
            "a retry reports no new card for the block the failed run wrote"
        );
        assert_eq!(1, alix::personal::read(&path, "deck-d").cards.len());
        let deck = alix::deck::Deck::load(&path).unwrap();
        assert!(
            alix::assemble::personal_cards(&deck)
                .cards
                .iter()
                .any(|card| card.id().as_deref() == Some(id.as_str())),
            "the card written before the failed save is still loaded for review"
        );
    }

    #[test]
    fn apply_exam_remediation_does_not_call_a_preexisting_sidecar_card_new() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let remediation = "## Why b?\nbecause\n";
        let mut other_device = alix::store::Store::open(root.join("other-device.json")).unwrap();
        assert_eq!(
            1,
            alix::store::store_remediation_cards(
                &mut other_device,
                Some(&path),
                "deck-d",
                "topic-a",
                &std::collections::HashSet::new(),
                remediation,
                None,
            )
            .into_result()
            .unwrap(),
            "another device creates the shared personal card"
        );
        let id = alix::personal::read(&path, "deck-d").cards[0]
            .card
            .id()
            .expect("the shared sidecar card is stamped");

        let displayed_as_new = apply_exam_remediation(
            path.to_string_lossy().into_owned(),
            root_arg(root),
            "topic-a".to_string(),
            remediation.to_string(),
            None,
        )
        .unwrap();

        assert_eq!(
            0, displayed_as_new,
            "the phone must not tell the user an already-shared card is new"
        );
        assert_eq!(
            1,
            alix::personal::read(&path, "deck-d").cards.len(),
            "the retry writes no card"
        );
        assert!(
            reopened(&path, root).progress(&id).is_none(),
            "the retry schedules nothing for the card"
        );
    }

    #[test]
    fn apply_exam_remediation_skips_a_block_equal_to_a_cloze_block() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = root.join("d.md");
        let block =
            "## Complete the quote\nTo be or not to be\n<!-- blank: span hidden=\"be\" -->\n";
        std::fs::write(
            &path,
            format!("---\nformat-version: 1\nid: deck-d\n---\n{block}"),
        )
        .unwrap();
        alix::stamp::stamp_deck(&path).unwrap();

        let created = apply_exam_remediation(
            path.to_string_lossy().into_owned(),
            root_arg(root),
            "deck-d".to_string(),
            block.to_string(),
            None,
        )
        .unwrap();

        assert_eq!(0, created, "the block key, not the cloze question, dedups");
        assert!(
            !alix::personal::sidecar_path(&path).exists(),
            "nothing reached disk"
        );
    }
}
