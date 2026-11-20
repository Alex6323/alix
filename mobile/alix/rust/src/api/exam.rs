use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExamStamp {
    pub sourced: String,
    pub unsourced_cards: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExamStart {
    pub deck_id: String,
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
        deck_id: deck.deck_token.context("the deck is not initialized")?,
        cards,
        eligible: start.eligible,
        stamp: ExamStamp {
            sourced: start.stamp.sourced,
            unsourced_cards: start.stamp.unsourced_cards,
        },
        unsourced_topics: start.unsourced_topics,
    })
}

#[flutter_rust_bridge::frb(sync)]
pub fn exam_deck_id(deck_path: String) -> Result<Option<String>> {
    alix::parser::read_deck_identity(Path::new(&deck_path))
        .with_context(|| format!("cannot read {deck_path}"))
}

fn held_path(root_dir: &str, deck_id: &str) -> PathBuf {
    alix::state::UserFiles::new(alix::workspace::root_store_path(Path::new(root_dir)))
        .exam_carried_for(deck_id)
}

fn load_held(path: &Path) -> Result<alix::exam_carried::HeldExam> {
    alix::exam_carried::HeldExam::load(path)?.context("no exam is held for this deck")
}

#[flutter_rust_bridge::frb(sync)]
pub fn held_exam_receive(root_dir: String, carried_json: String, start: ExamStart) -> Result<()> {
    let held = alix::exam_carried::HeldExam::received(
        &carried_json,
        alix::exam_carried::HeldStart {
            eligible: start.eligible,
            sourced: start.stamp.sourced,
            unsourced_cards: start.stamp.unsourced_cards,
            unsourced_topics: start.unsourced_topics,
        },
    )?;
    held.create(&held_path(&root_dir, &start.deck_id))
}

#[flutter_rust_bridge::frb(sync)]
pub fn held_exam_sitting(root_dir: String, deck_id: String) -> Result<Option<String>> {
    let held = alix::exam_carried::HeldExam::load(&held_path(&root_dir, &deck_id))?;
    Ok(held.map(|held| held.exam.sitting))
}

#[flutter_rust_bridge::frb(sync)]
pub fn held_exam_view(root_dir: String, deck_id: String) -> Result<Option<String>> {
    alix::exam_carried::HeldExam::load(&held_path(&root_dir, &deck_id))?
        .map(|held| serde_json::to_string(&held.view()).context("cannot encode the held exam"))
        .transpose()
}

#[flutter_rust_bridge::frb(sync)]
pub fn held_exam_save_answers(
    root_dir: String,
    deck_id: String,
    answers: Vec<String>,
    locked: Vec<bool>,
) -> Result<()> {
    let path = held_path(&root_dir, &deck_id);
    let mut held = load_held(&path)?;
    held.set_answers(answers, locked)?;
    held.save(&path)
}

#[flutter_rust_bridge::frb(sync)]
pub fn held_exam_material_changed(deck_path: String, root_dir: String) -> Result<bool> {
    let deck = alix::deck::Deck::load(Path::new(&deck_path))?;
    let deck_id = deck
        .deck_token
        .clone()
        .context("the deck is not initialized")?;
    let held = load_held(&held_path(&root_dir, &deck_id))?;
    Ok(!held.material_unchanged(&deck))
}

#[flutter_rust_bridge::frb(sync)]
pub fn held_exam_grade_request(root_dir: String, deck_id: String) -> Result<String> {
    load_held(&held_path(&root_dir, &deck_id))?
        .grade_request()?
        .to_json()
}

#[flutter_rust_bridge::frb(sync)]
pub fn held_exam_remove(root_dir: String, deck_id: String) -> Result<()> {
    let path = held_path(&root_dir, &deck_id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("cannot remove {}", path.display())),
    }
}

#[flutter_rust_bridge::frb(sync)]
pub fn held_exam_store_result(
    root_dir: String,
    deck_id: String,
    reply_json: String,
    received_ms: u64,
) -> Result<()> {
    let reply: alix::exam_carried::GradingReply =
        serde_json::from_str(&reply_json).context("unreadable grading reply")?;
    let path = held_path(&root_dir, &deck_id);
    let mut held = load_held(&path)?;
    held.store_result(&reply, received_ms)?;
    held.save(&path)
}

struct HeldApply {
    deck_path: PathBuf,
    root_dir: String,
    deck: alix::deck::Deck,
    deck_id: String,
    held_path: PathBuf,
    held: alix::exam_carried::HeldExam,
}

impl HeldApply {
    fn open(deck_path: &str, root_dir: &str) -> Result<Self> {
        let deck_path = PathBuf::from(deck_path);
        let deck = alix::deck::Deck::load(&deck_path)?;
        let deck_id = deck
            .deck_token
            .clone()
            .context("the deck is not initialized")?;
        let held_path = held_path(root_dir, &deck_id);
        let held = load_held(&held_path)?;
        Ok(Self {
            deck_path,
            root_dir: root_dir.to_string(),
            deck,
            deck_id,
            held_path,
            held,
        })
    }

    fn result(&self) -> Result<&alix::exam_carried::HeldResult> {
        self.held
            .result
            .as_ref()
            .context("no result is stored for this exam")
    }

    fn stamp(&self) -> alix::store::ExamStamp {
        alix::store::ExamStamp {
            sourced: self.held.start.sourced.clone(),
            unsourced_cards: self.held.start.unsourced_cards.clone(),
        }
    }
}

fn apply_progress(apply: &HeldApply, device: Option<String>) -> Result<alix::store::Store> {
    let received_ms = apply.result()?.received_ms;
    let record = apply
        .held
        .sitting_record()
        .context("no result is stored for this exam")?;
    let mut store = open_store(&apply.deck_path, &apply.root_dir, device)?;
    alix::exam_status::record_sitting_checked(
        &mut store,
        &apply.deck,
        &alix::exam_status::examined_topics(&apply.deck),
        &record,
        received_ms,
        apply.held.start.eligible,
        &apply.stamp(),
    )?;
    store.save()?;
    Ok(store)
}

fn apply_ideas(apply: &HeldApply) -> Result<()> {
    let received_ms = apply.result()?.received_ms;
    let mut ideas = apply.held.exam.ideas.clone();
    ideas.record(&apply.held.asked(), received_ms);
    let root_store = alix::workspace::root_store_path(Path::new(&apply.root_dir));
    ideas.save(&alix::state::UserFiles::new(root_store).exam_ideas_for(&apply.deck_id))
}

fn write_outcome(apply: &mut HeldApply, outcome: alix::exam_carried::HeldOutcome) -> Result<()> {
    apply.held.outcome = Some(outcome);
    apply.held.save(&apply.held_path)
}

#[flutter_rust_bridge::frb(sync)]
pub fn held_exam_apply(
    deck_path: String,
    root_dir: String,
    device: Option<String>,
) -> Result<String> {
    let mut apply = HeldApply::open(&deck_path, &root_dir)?;
    let received_ms = apply.result()?.received_ms;
    if apply.held.outcome.is_none() {
        if apply.held.material_unchanged(&apply.deck) {
            let store = apply_progress(&apply, device)?;
            apply_ideas(&apply)?;
            let standing = alix::exam_status::exam_high_mark(&store, &apply.deck_id, received_ms)
                .map(|text| alix::exam_carried::HeldStanding {
                    text,
                    exam_changed: alix::exam_status::standing_changed(
                        &store,
                        &apply.deck_id,
                        &apply.stamp(),
                    ),
                });
            write_outcome(
                &mut apply,
                alix::exam_carried::HeldOutcome {
                    recorded: true,
                    standing,
                },
            )?;
        } else {
            write_outcome(
                &mut apply,
                alix::exam_carried::HeldOutcome {
                    recorded: false,
                    standing: None,
                },
            )?;
        }
    }
    serde_json::to_string(&apply.held.view()).context("cannot encode the held exam")
}

#[flutter_rust_bridge::frb(sync)]
pub fn held_exam_remediation_request(root_dir: String, deck_id: String) -> Result<Option<String>> {
    load_held(&held_path(&root_dir, &deck_id))?
        .remediation_request()
        .map(|request| request.to_json())
        .transpose()
}

#[flutter_rust_bridge::frb(sync)]
pub fn held_exam_store_remediation(
    deck_path: String,
    root_dir: String,
    reply_json: String,
    device: Option<String>,
) -> Result<u32> {
    let mut apply = HeldApply::open(&deck_path, &root_dir)?;
    let reply: alix::exam_carried::GradingReply =
        serde_json::from_str(&reply_json).context("unreadable remediation reply")?;
    let cards = apply.held.remediation_cards(&reply)?;
    let fingerprints = apply
        .deck
        .cards
        .iter()
        .map(|card| card.block_fingerprint)
        .collect();
    let mut store = open_store(&apply.deck_path, &root_dir, device)?;
    let retire_after_days = alix::exam_status::retire_after_days_for(
        alix::config::ReviewConfig::default(),
        &apply.deck_path,
    );
    let mut total = 0;
    for (topic_id, text) in cards {
        let count = alix::store::store_remediation_cards(
            &mut store,
            Some(&apply.deck_path),
            &apply.deck_id,
            &topic_id,
            &fingerprints,
            text,
            retire_after_days,
        )
        .into_result()? as u32;
        apply.held.remediated.insert(topic_id, count);
        apply.held.save(&apply.held_path)?;
        total += count;
    }
    Ok(total)
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

    fn root_arg(root: &Path) -> String {
        root.to_string_lossy().into_owned()
    }

    #[test]
    fn exam_ideas_load_reads_the_document_the_phone_store_holds_for_the_deck() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        assert_eq!(
            None,
            exam_ideas_load(deck_arg(&path), root_arg(root)).unwrap(),
            "no document before one is saved"
        );

        let document = alix::exam_ideas::IdeasDocument::parse(
            r#"{"version":1,"topics":{"topic-a":{"fingerprint":"f","ideas":[]}}}"#,
        )
        .unwrap();
        document
            .save(&user_files(root).exam_ideas_for("deck-d"))
            .unwrap();

        assert_eq!(
            Some(document.to_json().unwrap()),
            exam_ideas_load(deck_arg(&path), root_arg(root)).unwrap(),
            "the saved document comes back"
        );
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

    fn held_file(root: &Path, deck_id: &str) -> PathBuf {
        root.join(format!(".alix/exam/{deck_id}.carried.json"))
    }

    fn carried_exam(topics: &[(String, String)], sitting: &str) -> alix::exam_carried::CarriedExam {
        alix::exam_carried::CarriedExam {
            version: alix::exam_carried::VERSION,
            sitting: sitting.to_string(),
            deck: "Deck D".to_string(),
            strictness: alix::config::Strictness::Balanced,
            full: true,
            unknown_cards: Vec::new(),
            topics: topics
                .iter()
                .map(|(topic_id, fingerprint)| alix::exam_carried::CarriedTopic {
                    topic_id: topic_id.clone(),
                    fingerprint: fingerprint.clone(),
                    label: topic_id.clone(),
                    cards: Vec::new(),
                })
                .collect(),
            questions: topics
                .iter()
                .map(|(topic_id, _)| alix::exam_carried::CarriedQuestion {
                    topic_id: topic_id.clone(),
                    idea: 0,
                    prompt: format!("question for {topic_id}?"),
                    points: vec!["a point".to_string()],
                    shape: alix::exam_carried::Shape::Instance,
                    mental_model: "a model".to_string(),
                })
                .collect(),
            ideas: alix::exam_ideas::IdeasDocument::parse(r#"{"version":1,"topics":{}}"#).unwrap(),
        }
    }

    fn received_json(exam: &alix::exam_carried::CarriedExam) -> String {
        serde_json::json!({ "exam": exam, "questions_units": exam.questions_units() }).to_string()
    }

    fn named_topics(ids: &[&str]) -> Vec<(String, String)> {
        ids.iter()
            .map(|id| (id.to_string(), format!("fingerprint of {id}")))
            .collect()
    }

    fn deck_topics(path: &Path) -> Vec<(String, String)> {
        let deck = alix::deck::Deck::load(path).unwrap();
        alix::exam_status::examined_topics(&deck)
            .into_iter()
            .map(|topic| (topic.topic_id, topic.fingerprint))
            .collect()
    }

    fn plain_start(deck_id: &str) -> ExamStart {
        ExamStart {
            deck_id: deck_id.to_string(),
            cards: None,
            eligible: true,
            stamp: ExamStamp {
                sourced: "sourced stamp".to_string(),
                unsourced_cards: vec!["card-u".to_string()],
            },
            unsourced_topics: vec!["Topic U".to_string()],
        }
    }

    fn receive_for(
        root: &Path,
        deck_id: &str,
        exam: &alix::exam_carried::CarriedExam,
    ) -> Result<()> {
        held_exam_receive(root_arg(root), received_json(exam), plain_start(deck_id))
    }

    fn deck_arg(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn receive_writes_the_held_file_by_root_and_deck_id_without_a_deck_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let exam = carried_exam(&named_topics(&["topic-a"]), "sitting-1");

        receive_for(root, "deck-d", &exam).unwrap();

        let held = alix::exam_carried::HeldExam::load(&held_file(root, "deck-d"))
            .unwrap()
            .expect("the held file was written");
        assert_eq!(exam, held.exam, "the document is held as received");
        assert_eq!(
            alix::exam_carried::HeldStart {
                eligible: true,
                sourced: "sourced stamp".to_string(),
                unsourced_cards: vec!["card-u".to_string()],
                unsourced_topics: vec!["Topic U".to_string()],
            },
            held.start,
            "the start is copied into the held file"
        );
        assert_eq!(
            Some("sitting-1".to_string()),
            held_exam_sitting(root_arg(root), "deck-d".to_string()).unwrap(),
            "the sitting is found by root and deck id"
        );
        assert_eq!(
            None,
            held_exam_sitting(root_arg(root), "deck-other".to_string()).unwrap(),
            "another deck id holds nothing"
        );
    }

    #[test]
    fn receive_refuses_a_deck_that_already_holds_an_exam_and_leaves_it_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        receive_for(
            root,
            "deck-d",
            &carried_exam(&named_topics(&["topic-a"]), "first"),
        )
        .unwrap();
        let before = std::fs::read(held_file(root, "deck-d")).unwrap();

        let error = receive_for(
            root,
            "deck-d",
            &carried_exam(&named_topics(&["topic-b"]), "second"),
        )
        .unwrap_err();

        assert!(
            format!("{error:#}").contains("already held"),
            "the library's refusal is passed through: {error:#}"
        );
        assert_eq!(
            before,
            std::fs::read(held_file(root, "deck-d")).unwrap(),
            "the held file is untouched"
        );
    }

    #[test]
    fn view_is_none_without_a_file_and_shows_the_questions_after_receive() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(
            None,
            held_exam_view(root_arg(root), "deck-d".to_string()).unwrap(),
            "no file, no view"
        );

        let exam = carried_exam(&named_topics(&["topic-a", "topic-b"]), "sitting-1");
        receive_for(root, "deck-d", &exam).unwrap();

        let json = held_exam_view(root_arg(root), "deck-d".to_string())
            .unwrap()
            .expect("a view after receive");
        let view: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!("sitting-1", view["sitting"], "view: {json}");
        assert_eq!(
            serde_json::json!(["question for topic-a?", "question for topic-b?"]),
            view["questions"],
            "view: {json}"
        );
    }

    #[test]
    fn saved_answers_and_locks_are_there_after_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        receive_for(
            root,
            "deck-d",
            &carried_exam(&named_topics(&["topic-a", "topic-b"]), "s"),
        )
        .unwrap();

        held_exam_save_answers(
            root_arg(root),
            "deck-d".to_string(),
            vec!["locked answer".to_string(), "a draft".to_string()],
            vec![true, false],
        )
        .unwrap();

        let held = alix::exam_carried::HeldExam::load(&held_file(root, "deck-d"))
            .unwrap()
            .unwrap();
        assert_eq!(
            vec!["locked answer", "a draft"],
            held.answers,
            "answers in the held file after the save"
        );
        assert_eq!(
            vec![true, false],
            held.locked,
            "locks in the held file after the save"
        );
        let view: serde_json::Value = serde_json::from_str(
            &held_exam_view(root_arg(root), "deck-d".to_string())
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            serde_json::json!(["locked answer", "a draft"]),
            view["answers"],
            "answers in the view after the save: {view}"
        );
        assert_eq!(
            serde_json::json!([true, false]),
            view["locked"],
            "locks in the view after the save: {view}"
        );
    }

    #[test]
    fn a_lock_over_the_answer_limit_is_refused_with_the_too_long_line_and_nothing_changes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        receive_for(
            root,
            "deck-d",
            &carried_exam(&named_topics(&["topic-a"]), "s"),
        )
        .unwrap();
        held_exam_save_answers(
            root_arg(root),
            "deck-d".to_string(),
            vec!["kept".into()],
            vec![false],
        )
        .unwrap();
        let before = std::fs::read(held_file(root, "deck-d")).unwrap();

        let error = held_exam_save_answers(
            root_arg(root),
            "deck-d".to_string(),
            vec!["x".repeat(alix::exam_carried::MAX_ANSWERS_BYTES + 1)],
            vec![true],
        )
        .unwrap_err();

        assert_eq!(
            alix::exam_carried::ANSWER_TOO_LONG,
            error.to_string(),
            "the refused lock passes the library's text through"
        );
        assert_eq!(
            before,
            std::fs::read(held_file(root, "deck-d")).unwrap(),
            "the held file's bytes are unchanged"
        );
    }

    #[test]
    fn material_changed_is_false_for_the_deck_as_started_and_true_after_an_examined_card_changed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let start = exam_sitting_start(deck_arg(&path), root_arg(root), false, NOW).unwrap();
        let exam = carried_exam(&deck_topics(&path), "s");
        held_exam_receive(root_arg(root), received_json(&exam), start).unwrap();
        assert!(
            !held_exam_material_changed(deck_arg(&path), root_arg(root)).unwrap(),
            "the deck as started"
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

        assert!(
            held_exam_material_changed(deck_arg(&path), root_arg(root)).unwrap(),
            "an examined topic's source changed"
        );
    }

    #[test]
    fn two_grade_requests_for_unchanged_answers_are_byte_equal_and_equal_the_lib_builders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let exam = carried_exam(&named_topics(&["topic-a", "topic-b"]), "s");
        receive_for(root, "deck-d", &exam).unwrap();
        let answers = vec!["first answer".to_string(), "   ".to_string()];
        held_exam_save_answers(
            root_arg(root),
            "deck-d".to_string(),
            answers.clone(),
            vec![true, true],
        )
        .unwrap();

        let first = held_exam_grade_request(root_arg(root), "deck-d".to_string()).unwrap();
        let second = held_exam_grade_request(root_arg(root), "deck-d".to_string()).unwrap();

        assert_eq!(first, second, "a repeat sends the same bytes");
        assert_eq!(
            exam.grade_request(&answers).to_json().unwrap(),
            first,
            "the bridge sends what the library builds"
        );
    }

    #[test]
    fn remove_deletes_the_file_and_an_absent_file_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        receive_for(
            root,
            "deck-d",
            &carried_exam(&named_topics(&["topic-a"]), "s"),
        )
        .unwrap();
        assert!(
            held_file(root, "deck-d").is_file(),
            "control: the file exists"
        );

        held_exam_remove(root_arg(root), "deck-d".to_string()).unwrap();
        assert!(!held_file(root, "deck-d").exists(), "the file is gone");
        held_exam_remove(root_arg(root), "deck-d".to_string()).unwrap();

        assert_eq!(
            None,
            held_exam_view(root_arg(root), "deck-d".to_string()).unwrap(),
            "no view once the file is removed"
        );
    }

    #[test]
    fn a_sitting_start_names_the_deck_id() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);

        let start = exam_sitting_start(deck_arg(&path), root_arg(root), false, NOW).unwrap();

        assert_eq!(
            "deck-d", start.deck_id,
            "the sitting start carries the deck's id"
        );
    }

    #[test]
    fn the_deck_id_is_the_one_a_sitting_start_carries_and_none_for_a_deck_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let start = exam_sitting_start(deck_arg(&path), root_arg(root), false, NOW).unwrap();
        let renamed = path.with_file_name("renamed.md");
        std::fs::rename(&path, &renamed).unwrap();
        let bare = root.join("bare.md");
        std::fs::write(&bare, "## q\na\n").unwrap();

        assert_eq!(
            Some(start.deck_id),
            exam_deck_id(deck_arg(&renamed)).unwrap(),
            "the renamed deck file declares the id its sitting start carried"
        );
        assert_eq!(
            None,
            exam_deck_id(deck_arg(&bare)).unwrap(),
            "a deck file without frontmatter declares no id"
        );
        assert!(
            exam_deck_id(deck_arg(&root.join("absent.md"))).is_err(),
            "a deck file that cannot be read is an error"
        );
    }

    #[test]
    fn receive_and_save_answers_write_no_ideas_document() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        receive_for(
            root,
            "deck-d",
            &carried_exam(&named_topics(&["topic-a"]), "s"),
        )
        .unwrap();
        held_exam_save_answers(
            root_arg(root),
            "deck-d".to_string(),
            vec!["a".into()],
            vec![true],
        )
        .unwrap();

        let mut files: Vec<String> = std::fs::read_dir(root.join(".alix/exam"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        assert_eq!(
            vec!["deck-d.carried.json".to_string()],
            files,
            "the held file is the only file in the exam folder"
        );
    }

    #[test]
    fn another_file_in_the_exam_folder_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let exam_folder = root.join(".alix/exam");
        std::fs::create_dir_all(&exam_folder).unwrap();
        let siblings = [
            (exam_folder.join("deck-d.prepared.json"), "prepared bytes"),
            (exam_folder.join("deck-d.json"), "ideas bytes"),
        ];
        for (sibling, bytes) in &siblings {
            std::fs::write(sibling, bytes).unwrap();
        }

        receive_for(
            root,
            "deck-d",
            &carried_exam(&named_topics(&["topic-a"]), "s"),
        )
        .unwrap();
        held_exam_save_answers(
            root_arg(root),
            "deck-d".to_string(),
            vec!["a".into()],
            vec![true],
        )
        .unwrap();
        held_exam_remove(root_arg(root), "deck-d".to_string()).unwrap();

        for (sibling, bytes) in &siblings {
            assert_eq!(
                *bytes,
                std::fs::read_to_string(sibling).unwrap(),
                "{} keeps its bytes",
                sibling.display()
            );
        }
    }

    #[test]
    fn a_held_exam_is_reachable_by_root_and_deck_id_with_no_deck_file_anywhere() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let exam = carried_exam(&named_topics(&["topic-a", "topic-b"]), "sitting-1");
        held_exam_receive(root_arg(root), received_json(&exam), plain_start("deck-d")).unwrap();
        assert!(
            std::fs::read_dir(root).unwrap().all(|entry| entry
                .unwrap()
                .path()
                .extension()
                .is_none_or(|ext| ext != "md")),
            "control: no deck file exists under the root"
        );

        let view: serde_json::Value = serde_json::from_str(
            &held_exam_view(root_arg(root), "deck-d".to_string())
                .unwrap()
                .expect("a view without a deck"),
        )
        .unwrap();
        assert_eq!(
            serde_json::json!(["question for topic-a?", "question for topic-b?"]),
            view["questions"],
            "view of the received exam without a deck: {view}"
        );
        let answers = vec!["first".to_string(), "second".to_string()];
        held_exam_save_answers(
            root_arg(root),
            "deck-d".to_string(),
            answers.clone(),
            vec![true, false],
        )
        .unwrap();
        let reloaded: serde_json::Value = serde_json::from_str(
            &held_exam_view(root_arg(root), "deck-d".to_string())
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            (serde_json::json!(answers), serde_json::json!([true, false])),
            (reloaded["answers"].clone(), reloaded["locked"].clone()),
            "answers and locks after a reload without a deck: {reloaded}"
        );
        assert_eq!(
            exam.grade_request(&answers).to_json().unwrap(),
            held_exam_grade_request(root_arg(root), "deck-d".to_string()).unwrap(),
            "the grade request is built without a deck"
        );
        held_exam_remove(root_arg(root), "deck-d".to_string()).unwrap();
        assert!(
            !held_file(root, "deck-d").exists(),
            "remove deleted {} without a deck",
            held_file(root, "deck-d").display()
        );
    }

    const RECEIVED: u64 = NOW + 60_000;

    fn user_files(root: &Path) -> alix::state::UserFiles {
        alix::state::UserFiles::new(alix::workspace::root_store_path(root))
    }

    fn progress_bytes(root: &Path) -> Option<Vec<u8>> {
        std::fs::read(user_files(root).progress_for("deck-d")).ok()
    }

    fn ideas_bytes(root: &Path) -> Option<Vec<u8>> {
        std::fs::read(user_files(root).exam_ideas_for("deck-d")).ok()
    }

    fn held_now(root: &Path) -> alix::exam_carried::HeldExam {
        alix::exam_carried::HeldExam::load(&held_file(root, "deck-d"))
            .unwrap()
            .expect("the held file exists")
    }

    fn untested_ideas(exam: &alix::exam_carried::CarriedExam) -> alix::exam_ideas::IdeasDocument {
        alix::exam_ideas::IdeasDocument {
            version: alix::exam_ideas::VERSION,
            topics: exam
                .topics
                .iter()
                .map(|topic| {
                    (
                        topic.topic_id.clone(),
                        alix::exam_ideas::IdeaTopic {
                            fingerprint: topic.fingerprint.clone(),
                            ideas: vec![alix::exam_ideas::Idea {
                                text: format!("the idea of {}", topic.topic_id),
                                ..Default::default()
                            }],
                        },
                    )
                })
                .collect(),
        }
    }

    fn held_on_deck(root: &Path) -> (PathBuf, alix::exam_carried::CarriedExam) {
        let path = two_topic_deck(root);
        let start = exam_sitting_start(deck_arg(&path), root_arg(root), false, NOW).unwrap();
        let mut exam = carried_exam(&deck_topics(&path), "sitting-1");
        exam.ideas = untested_ideas(&exam);
        held_exam_receive(root_arg(root), received_json(&exam), start).unwrap();
        (path, exam)
    }

    fn grading_reply(
        exam: &alix::exam_carried::CarriedExam,
        topics: &[(bool, &[&str])],
    ) -> alix::exam_carried::GradingReply {
        let request = exam.grade_request(&vec![String::new(); exam.questions.len()]);
        let verdict = |pass: bool| {
            if pass {
                alix::exam_carried::Verdict::Pass.label()
            } else {
                alix::exam_carried::Verdict::Fail.label()
            }
        };
        let passed = topics.iter().filter(|(pass, _)| *pass).count() as u32;
        alix::exam_carried::GradingReply {
            sitting: Some(exam.sitting.clone()),
            phase: "results".to_string(),
            grades: request
                .questions
                .iter()
                .map(|question| alix::exam_carried::GradedAnswer {
                    topic: question.topic,
                    verdict: verdict(topics[question.topic].0).to_string(),
                    feedback: String::new(),
                    feedback_units: Vec::new(),
                    missed: Vec::new(),
                    missed_units: Vec::new(),
                    points_units: Vec::new(),
                })
                .collect(),
            topics: topics
                .iter()
                .map(|(pass, gaps)| alix::exam_carried::GradedTopic {
                    pass: *pass,
                    verdicts: alix::exam_carried::Verdicts {
                        pass: u32::from(*pass),
                        partial: 0,
                        total: 1,
                    },
                    gaps: gaps.iter().map(|gap| gap.to_string()).collect(),
                })
                .collect(),
            verdicts: Some(alix::exam_carried::Verdicts {
                pass: passed,
                partial: 0,
                total: topics.len() as u32,
            }),
            mastered: passed as usize == topics.len(),
            cards_by_topic: Vec::new(),
            thinking: false,
            elapsed: None,
            error: None,
            estimate: None,
        }
    }

    const BOTH_PASS: &[(bool, &[&str])] = &[(true, &[]), (true, &[])];
    const BOTH_FAIL: &[(bool, &[&str])] = &[(false, &["gap of a"]), (false, &["gap of b"])];

    fn store_reply(root: &Path, reply: &alix::exam_carried::GradingReply) -> Result<()> {
        held_exam_store_result(
            root_arg(root),
            "deck-d".to_string(),
            serde_json::to_string(reply).unwrap(),
            RECEIVED,
        )
    }

    fn change_the_examined_source(root: &Path, path: &Path) {
        let live = root.join("live").join("d.md.txt");
        std::fs::write(&live, "the changed source of d.md\n").unwrap();
        alix::sources::commands::update(
            path,
            &alix::sources::source_id(b"the source of d.md\n"),
            &live,
            None,
            false,
            &alix::config::ExamConfig::default(),
        )
        .unwrap();
    }

    fn remediation_reply(sitting: &str, cards: &[(usize, &str)]) -> String {
        serde_json::to_string(&alix::exam_carried::GradingReply {
            sitting: Some(sitting.to_string()),
            phase: "remediated".to_string(),
            grades: Vec::new(),
            topics: Vec::new(),
            verdicts: None,
            mastered: false,
            cards_by_topic: cards
                .iter()
                .map(|(topic, text)| alix::exam_carried::TopicNumberCards {
                    topic: *topic,
                    cards: text.to_string(),
                })
                .collect(),
            thinking: false,
            elapsed: None,
            error: None,
            estimate: None,
        })
        .unwrap()
    }

    fn remediation_topics(root: &Path) -> Option<Vec<usize>> {
        held_exam_remediation_request(root_arg(root), "deck-d".to_string())
            .unwrap()
            .map(|json| {
                alix::exam_carried::RemediationRequest::parse(json.as_bytes())
                    .unwrap()
                    .topics
                    .into_iter()
                    .map(|topic| topic.topic)
                    .collect()
            })
    }

    fn outcome_of(view_json: &str) -> Option<alix::exam_carried::HeldOutcome> {
        let view: serde_json::Value = serde_json::from_str(view_json).unwrap();
        serde_json::from_value(view["outcome"].clone()).unwrap()
    }

    #[derive(Debug, PartialEq)]
    struct ExamFields {
        topic_a: Option<alix::store::ExamRecord>,
        topic_b: Option<alix::store::ExamRecord>,
        full: Option<alix::store::FullExamResult>,
        passed_at: Option<u64>,
        mastered_at: Option<u64>,
        stamp: Option<alix::store::ExamStamp>,
        ideas: Option<String>,
        outcome: Option<alix::exam_carried::HeldOutcome>,
    }

    fn exam_fields(path: &Path, root: &Path) -> ExamFields {
        let store = reopened(path, root);
        ExamFields {
            topic_a: store.topic_result("deck-d", "topic-a").cloned(),
            topic_b: store.topic_result("deck-d", "topic-b").cloned(),
            full: store.full_exam_result("deck-d"),
            passed_at: store.deck_passed_at("deck-d"),
            mastered_at: store.deck_mastered_at("deck-d"),
            stamp: store.exam_stamp("deck-d").cloned(),
            ideas: std::fs::read_to_string(user_files(root).exam_ideas_for("deck-d")).ok(),
            outcome: held_now(root).outcome,
        }
    }

    #[test]
    fn store_result_writes_the_reply_and_its_time_and_a_second_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (_, exam) = held_on_deck(root);
        let reply = grading_reply(&exam, &[(true, &[]), (false, &["gap of b"])]);

        store_reply(root, &reply).unwrap();

        let result = held_now(root)
            .result
            .expect("a result in the held file after the store");
        assert_eq!(
            RECEIVED, result.received_ms,
            "the stored time after the store"
        );
        assert_eq!(
            (reply.grades.clone(), reply.topics.clone()),
            (result.grades.clone(), result.topics.clone()),
            "the stored grades and topics equal the reply's"
        );
        assert_eq!(None, held_now(root).outcome, "storing writes no outcome");
        let before = std::fs::read(held_file(root, "deck-d")).unwrap();

        let second = store_reply(root, &grading_reply(&exam, BOTH_PASS));

        assert!(second.is_err(), "a second store is refused: {second:?}");
        assert_eq!(
            before,
            std::fs::read(held_file(root, "deck-d")).unwrap(),
            "the held file keeps the first result's bytes after a refused second"
        );
    }

    #[test]
    fn apply_records_progress_stamps_the_tested_ideas_and_writes_the_standing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, exam) = held_on_deck(root);
        store_reply(root, &grading_reply(&exam, BOTH_PASS)).unwrap();

        let view = held_exam_apply(deck_arg(&path), root_arg(root), None).unwrap();

        let store = reopened(&path, root);
        for topic in ["topic-a", "topic-b"] {
            assert_eq!(
                Some((RECEIVED, true)),
                store
                    .topic_result("deck-d", topic)
                    .map(|record| (record.at_ms, record.pass)),
                "{topic} is recorded at the stored time"
            );
        }
        assert_eq!(
            (Some(RECEIVED), Some(RECEIVED), Some(RECEIVED)),
            (
                store.full_exam_result("deck-d").map(|result| result.at_ms),
                store.deck_passed_at("deck-d"),
                store.deck_mastered_at("deck-d")
            ),
            "full count, passed and mastered at the stored time"
        );
        let held = held_now(root);
        assert_eq!(
            Some(&alix::store::ExamStamp {
                sourced: held.start.sourced.clone(),
                unsourced_cards: held.start.unsourced_cards.clone(),
            }),
            store.exam_stamp("deck-d"),
            "the start's stamp is written with the standing"
        );
        let ideas =
            alix::exam_ideas::IdeasDocument::load(&user_files(root).exam_ideas_for("deck-d"))
                .unwrap()
                .expect("the deck's ideas document after the apply");
        for topic in ["topic-a", "topic-b"] {
            let idea = &ideas.topics[topic].ideas[0];
            assert_eq!(
                (Some(RECEIVED), Some(true)),
                (idea.last_tested_ms, idea.last_test_passed),
                "{topic}'s tested idea is stamped at the stored time"
            );
        }
        let outcome = held.outcome.clone().expect("an outcome after the apply");
        let standing = outcome
            .standing
            .clone()
            .expect("a standing after a full pass");
        assert!(
            outcome.recorded && standing.text.starts_with("mastered (") && !standing.exam_changed,
            "the outcome is recorded with the standing: {outcome:?}"
        );
        assert_eq!(
            Some(outcome),
            outcome_of(&view),
            "the returned view carries the outcome: {view}"
        );

        let progress = progress_bytes(root);
        let ideas = ideas_bytes(root);
        let held = std::fs::read(held_file(root, "deck-d")).unwrap();
        let again = held_exam_apply(deck_arg(&path), root_arg(root), None).unwrap();
        assert_eq!(view, again, "a second apply returns the same view");
        assert_eq!(
            (progress, ideas, held),
            (
                progress_bytes(root),
                ideas_bytes(root),
                std::fs::read(held_file(root, "deck-d")).unwrap()
            ),
            "a second apply after the outcome writes nothing"
        );
    }

    #[test]
    fn a_kill_at_each_point_ends_like_an_uninterrupted_apply() {
        let control_dir = tempfile::tempdir().unwrap();
        let control_root = control_dir.path();
        let (control_path, exam) = held_on_deck(control_root);
        store_reply(control_root, &grading_reply(&exam, BOTH_PASS)).unwrap();
        held_exam_apply(deck_arg(&control_path), root_arg(control_root), None).unwrap();
        let control = exam_fields(&control_path, control_root);
        assert!(
            control.passed_at.is_some() && control.ideas.is_some(),
            "control: the uninterrupted apply recorded and stamped: {control:?}"
        );

        for (point, steps) in [
            ("after the result is stored", 0),
            ("after progress is saved", 1),
            ("after the ideas are saved", 2),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let (path, exam) = held_on_deck(root);
            store_reply(root, &grading_reply(&exam, BOTH_PASS)).unwrap();
            let apply = HeldApply::open(&deck_arg(&path), &root_arg(root)).unwrap();
            if steps >= 1 {
                apply_progress(&apply, None).unwrap();
            }
            if steps >= 2 {
                apply_ideas(&apply).unwrap();
            }
            assert_eq!(
                None,
                held_now(root).outcome,
                "killed {point}: no outcome yet"
            );

            held_exam_apply(deck_arg(&path), root_arg(root), None).unwrap();

            assert_eq!(
                control,
                exam_fields(&path, root),
                "killed {point}: the reopened apply ends like the uninterrupted one"
            );
            assert!(
                held_exam_grade_request(root_arg(root), "deck-d".to_string()).is_err(),
                "killed {point}: no grade request is built"
            );
        }
    }

    #[cfg(unix)]
    struct ReadOnly(PathBuf);

    #[cfg(unix)]
    impl ReadOnly {
        fn new(path: PathBuf) -> Self {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o555)).unwrap();
            Self(path)
        }
    }

    #[cfg(unix)]
    impl Drop for ReadOnly {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_apply_that_cannot_save_progress_writes_no_outcome_and_records_on_the_next_call() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, exam) = held_on_deck(root);
        store_reply(root, &grading_reply(&exam, BOTH_PASS)).unwrap();
        let progress = user_files(root).progress();
        std::fs::create_dir_all(&progress).unwrap();

        let guard = ReadOnly::new(progress);
        let first = held_exam_apply(deck_arg(&path), root_arg(root), None);
        drop(guard);

        assert!(
            first.is_err(),
            "the apply fails on a read-only progress folder: {first:?}"
        );
        assert_eq!(
            None,
            held_now(root).outcome,
            "a failed progress save writes no outcome"
        );
        assert_eq!(
            None,
            ideas_bytes(root),
            "a failed progress save stamps no idea"
        );

        let view = held_exam_apply(deck_arg(&path), root_arg(root), None).unwrap();

        assert_eq!(
            Some(true),
            outcome_of(&view).map(|outcome| outcome.recorded),
            "the next call records: {view}"
        );
        assert_eq!(
            Some(RECEIVED),
            reopened(&path, root)
                .topic_result("deck-d", "topic-a")
                .map(|record| record.at_ms),
            "the next call records at the stored time"
        );
    }

    #[test]
    fn an_apply_whose_ideas_cannot_be_saved_writes_no_outcome_and_stamps_on_the_next_call() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, exam) = held_on_deck(root);
        store_reply(root, &grading_reply(&exam, BOTH_PASS)).unwrap();
        let ideas = user_files(root).exam_ideas_for("deck-d");
        std::fs::create_dir_all(&ideas).unwrap();

        let first = held_exam_apply(deck_arg(&path), root_arg(root), None);
        std::fs::remove_dir(&ideas).unwrap();

        assert!(
            first.is_err(),
            "the apply fails where the ideas document cannot be written: {first:?}"
        );
        assert_eq!(
            None,
            held_now(root).outcome,
            "a failed ideas save writes no outcome"
        );

        held_exam_apply(deck_arg(&path), root_arg(root), None).unwrap();

        assert!(
            held_now(root)
                .outcome
                .is_some_and(|outcome| outcome.recorded),
            "the next call writes the recorded outcome"
        );
        assert!(
            ideas_bytes(root).is_some(),
            "the next call stamps the ideas"
        );
    }

    #[test]
    fn a_result_applied_after_the_deck_changed_records_nothing_stamps_no_idea_and_stores_not_recorded(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, exam) = held_on_deck(root);
        store_reply(root, &grading_reply(&exam, BOTH_PASS)).unwrap();
        change_the_examined_source(root, &path);
        let progress = progress_bytes(root);
        let ideas = ideas_bytes(root);

        let applied = held_exam_apply(deck_arg(&path), root_arg(root), None);

        assert_eq!(
            progress,
            progress_bytes(root),
            "the progress file's bytes are unchanged after {applied:?}"
        );
        assert_eq!(
            ideas,
            ideas_bytes(root),
            "the ideas file is unchanged after {applied:?}"
        );
        let view = applied.unwrap();
        assert_eq!(
            Some(alix::exam_carried::HeldOutcome {
                recorded: false,
                standing: None,
            }),
            outcome_of(&view),
            "a deck changed before the apply stores not recorded: {view}"
        );
    }

    #[test]
    fn a_deck_changed_since_the_start_is_stored_as_not_recorded_with_no_idea_stamped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, exam) = held_on_deck(root);
        store_reply(root, &grading_reply(&exam, BOTH_PASS)).unwrap();
        change_the_examined_source(root, &path);

        let view = held_exam_apply(deck_arg(&path), root_arg(root), None).unwrap();

        assert_eq!(
            Some(alix::exam_carried::HeldOutcome {
                recorded: false,
                standing: None,
            }),
            outcome_of(&view),
            "a changed deck is stored as not recorded: {view}"
        );
        assert_eq!(
            None,
            ideas_bytes(root),
            "no idea is stamped for a changed deck"
        );
        assert_eq!(
            None,
            reopened(&path, root).topic_result("deck-d", "topic-a"),
            "no topic result is recorded for a changed deck"
        );
    }

    fn applied_with_both_failed(root: &Path) -> (PathBuf, alix::exam_carried::CarriedExam) {
        let (path, exam) = held_on_deck(root);
        store_reply(root, &grading_reply(&exam, BOTH_FAIL)).unwrap();
        held_exam_apply(deck_arg(&path), root_arg(root), None).unwrap();
        (path, exam)
    }

    #[test]
    fn a_needs_work_start_names_only_the_cards_of_the_topic_the_recorded_result_failed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, exam) = held_on_deck(root);
        store_reply(
            root,
            &grading_reply(&exam, &[(true, &[]), (false, &["gap of b"])]),
        )
        .unwrap();
        held_exam_apply(deck_arg(&path), root_arg(root), None).unwrap();

        let start = exam_sitting_start(deck_arg(&path), root_arg(root), true, NOW).unwrap();

        assert_eq!(
            Some(vec!["card-b1".to_string()]),
            start.cards,
            "after A passed and B failed, the needs-work start names the card of B only"
        );
    }

    #[test]
    fn a_start_with_an_unreviewed_card_in_an_unsourced_topic_is_ineligible_and_names_that_topic() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = two_topic_deck(root);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}\n# C\n\n## c1\nunreviewed\n")).unwrap();

        let start = exam_sitting_start(deck_arg(&path), root_arg(root), false, NOW).unwrap();

        assert!(
            !start.eligible,
            "an unreviewed card in the unsourced topic C fails the review condition"
        );
        assert_eq!(
            vec!["C".to_string()],
            start.unsourced_topics,
            "the start names C as the topic the exam leaves out"
        );
    }

    fn personal_fronts(path: &Path) -> Vec<(String, String)> {
        let mut fronts: Vec<(String, String)> = alix::personal::read(path, "deck-d")
            .cards
            .into_iter()
            .map(|parsed| (parsed.topic_id, parsed.card.front))
            .collect();
        fronts.sort();
        fronts
    }

    #[test]
    fn remediation_cards_land_on_the_topic_whose_number_the_reply_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, exam) = applied_with_both_failed(root);
        let numbered = |topic: usize| held_now(root).topic_id_of(topic).map(str::to_string);
        assert_eq!(
            (Some("topic-a".to_string()), Some("topic-b".to_string())),
            (numbered(0), numbered(1)),
            "control: the exam numbers topic-a 0 and topic-b 1"
        );

        let stored = held_exam_store_remediation(
            deck_arg(&path),
            root_arg(root),
            remediation_reply(
                &exam.sitting,
                &[(1, "## Why b?\nbecause b\n"), (0, "## Why a?\nbecause a\n")],
            ),
            None,
        )
        .unwrap();

        assert_eq!(2, stored, "both card sets are stored");
        assert_eq!(
            vec![
                ("topic-a".to_string(), "Why a?".to_string()),
                ("topic-b".to_string(), "Why b?".to_string()),
            ],
            personal_fronts(&path),
            "each card lands on the topic its number names"
        );
        assert_eq!(
            std::collections::BTreeMap::from([
                ("topic-a".to_string(), 1),
                ("topic-b".to_string(), 1),
            ]),
            held_now(root).remediated,
            "both topics are noted as remediated"
        );
    }

    #[test]
    fn a_remediated_topic_is_left_out_of_the_next_remediation_request() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, exam) = applied_with_both_failed(root);
        assert_eq!(
            Some(vec![0, 1]),
            remediation_topics(root),
            "control: both failed topics are requested"
        );

        held_exam_store_remediation(
            deck_arg(&path),
            root_arg(root),
            remediation_reply(&exam.sitting, &[(0, "## Why a?\nbecause a\n")]),
            None,
        )
        .unwrap();

        assert_eq!(
            Some(vec![1]),
            remediation_topics(root),
            "the remediated topic 0 is left out of the next request"
        );
    }

    #[test]
    fn a_card_set_the_store_refuses_keeps_the_topics_stored_before_it_noted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, exam) = applied_with_both_failed(root);
        let noted = |root: &Path| held_now(root).remediated;

        let failed = held_exam_store_remediation(
            deck_arg(&path),
            root_arg(root),
            remediation_reply(
                &exam.sitting,
                &[(0, "## Why a?\nbecause a\n"), (1, "## q\n")],
            ),
            None,
        );

        assert!(
            failed.is_err(),
            "the reply whose second card set is malformed fails: {failed:?}"
        );
        assert_eq!(
            std::collections::BTreeMap::from([("topic-a".to_string(), 1)]),
            noted(root),
            "after the failure the first topic is noted with its count and the second is not"
        );
        assert_eq!(
            vec![("topic-a".to_string(), "Why a?".to_string())],
            personal_fronts(&path),
            "after the failure only the first topic's card is stored"
        );
        assert_eq!(
            Some(vec![1]),
            remediation_topics(root),
            "after the failure only the second topic is requested"
        );

        let stored = held_exam_store_remediation(
            deck_arg(&path),
            root_arg(root),
            remediation_reply(&exam.sitting, &[(1, "## Why b?\nbecause b\n")]),
            None,
        )
        .unwrap();

        assert_eq!(1, stored, "the second topic's reply stores its card");
        assert_eq!(
            std::collections::BTreeMap::from([
                ("topic-a".to_string(), 1),
                ("topic-b".to_string(), 1),
            ]),
            noted(root),
            "after the second reply both topics are noted, topic-a's count unchanged"
        );
        assert_eq!(
            None,
            remediation_topics(root),
            "after the second reply no topic is requested"
        );
        assert_eq!(
            vec![
                ("topic-a".to_string(), "Why a?".to_string()),
                ("topic-b".to_string(), "Why b?".to_string()),
            ],
            personal_fronts(&path),
            "after the second reply each front exists once"
        );
    }

    #[test]
    fn a_remediation_reply_for_a_result_that_was_not_recorded_is_refused_and_writes_no_card() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, exam) = held_on_deck(root);
        store_reply(root, &grading_reply(&exam, BOTH_FAIL)).unwrap();
        change_the_examined_source(root, &path);
        let view = held_exam_apply(deck_arg(&path), root_arg(root), None).unwrap();
        assert_eq!(
            Some(false),
            outcome_of(&view).map(|outcome| outcome.recorded),
            "control: the changed deck's result is not recorded: {view}"
        );

        let refused = held_exam_store_remediation(
            deck_arg(&path),
            root_arg(root),
            remediation_reply(&exam.sitting, &[(0, "## Why a?\nbecause a\n")]),
            None,
        );

        assert!(
            refused.is_err(),
            "a remediation reply for a result not recorded is refused: {refused:?}"
        );
        assert_eq!(
            Vec::<(String, String)>::new(),
            personal_fronts(&path),
            "the refused reply writes no card"
        );
    }

    #[test]
    fn a_deck_file_renamed_after_the_hand_off_is_still_graded_and_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, exam) = held_on_deck(root);
        let renamed = path.with_file_name("renamed.md");
        std::fs::rename(&path, &renamed).unwrap();
        store_reply(root, &grading_reply(&exam, BOTH_PASS)).unwrap();

        let view = held_exam_apply(deck_arg(&renamed), root_arg(root), None).unwrap();

        assert_eq!(
            Some(true),
            outcome_of(&view).map(|outcome| outcome.recorded),
            "the renamed deck's result is recorded: {view}"
        );
        assert_eq!(
            Some(RECEIVED),
            reopened(&renamed, root)
                .topic_result("deck-d", "topic-a")
                .map(|record| record.at_ms),
            "the renamed deck's progress carries the result"
        );
    }

    #[test]
    fn a_remediation_reply_for_another_sitting_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, _) = applied_with_both_failed(root);
        let before = std::fs::read(held_file(root, "deck-d")).unwrap();

        let refused = held_exam_store_remediation(
            deck_arg(&path),
            root_arg(root),
            remediation_reply("another-sitting", &[(0, "## Why a?\nbecause a\n")]),
            None,
        );

        assert!(
            refused
                .as_ref()
                .is_err_and(|error| error.to_string().contains("another sitting")),
            "a reply for another sitting is refused: {refused:?}"
        );
        assert_eq!(
            Vec::<(String, String)>::new(),
            personal_fronts(&path),
            "no card is stored"
        );
        assert_eq!(
            before,
            std::fs::read(held_file(root, "deck-d")).unwrap(),
            "the held file is unchanged"
        );
    }

    fn card_progress(root: &Path) -> serde_json::Map<String, serde_json::Value> {
        let bytes = progress_bytes(root).expect("the deck's progress file exists");
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        document["cards"]
            .as_object()
            .cloned()
            .expect("the progress file has a cards member")
    }

    fn held_with_a_reviewed_card(root: &Path) -> (PathBuf, alix::exam_carried::CarriedExam) {
        let (path, exam) = held_on_deck(root);
        let mut store = open_store(&path, &root_arg(root), None).unwrap();
        store.get_or_insert("card-a1").record_review(
            NOW,
            alix::scheduler::Grade::Pass,
            alix::depth::Depth::Recall,
            false,
        );
        store.save().unwrap();
        (path, exam)
    }

    #[test]
    fn an_apply_leaves_every_cards_review_progress_as_the_store_holds_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, exam) = held_with_a_reviewed_card(root);
        let before = card_progress(root);
        assert!(
            before.contains_key("card-a1"),
            "control: the recorded review is in the progress file before the apply: {before:?}"
        );

        store_reply(root, &grading_reply(&exam, BOTH_FAIL)).unwrap();
        held_exam_apply(deck_arg(&path), root_arg(root), None).unwrap();

        assert_eq!(
            before,
            card_progress(root),
            "the card progress after the apply equals the progress before it"
        );
    }

    #[test]
    fn a_remediation_that_revives_a_card_leaves_every_other_cards_review_progress_as_the_store_holds_it(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, exam) = held_with_a_reviewed_card(root);
        store_reply(root, &grading_reply(&exam, BOTH_FAIL)).unwrap();
        held_exam_apply(deck_arg(&path), root_arg(root), None).unwrap();
        let remediate = |topic: usize| {
            held_exam_store_remediation(
                deck_arg(&path),
                root_arg(root),
                remediation_reply(&exam.sitting, &[(topic, "## q\na\n")]),
                None,
            )
            .unwrap()
        };
        assert_eq!(1, remediate(0), "control: the first remediation stores its one card");
        let revived = alix::personal::read(&path, "deck-d").cards[0]
            .card
            .id()
            .expect("the remediation card has an id");
        {
            let mut store = open_store(&path, &root_arg(root), None).unwrap();
            store.get_or_insert(&revived).recall = Some(alix::store::FsrsState {
                state: 2,
                scheduled_days: alix::session::DEFAULT_RETIRE_AFTER_DAYS,
                ..Default::default()
            });
            store.save().unwrap();
        }
        let before = card_progress(root);
        assert!(
            before.contains_key("card-a1") && before.contains_key(&revived),
            "control: the reviewed card and the retired {revived} are in the progress file: {before:?}"
        );

        assert_eq!(
            1,
            remediate(1),
            "control: the second remediation revives the retired card"
        );

        let after = card_progress(root);
        assert_eq!(
            Some(&serde_json::to_value(alix::store::CardState::new()).unwrap()),
            after.get(&revived),
            "the revived {revived} starts over as a new card after the remediation"
        );
        for (card, progress) in before.iter().filter(|(card, _)| **card != revived) {
            assert_eq!(
                Some(progress),
                after.get(card),
                "{card}'s progress after the remediation that revives a card equals its progress before it"
            );
        }
    }
}
