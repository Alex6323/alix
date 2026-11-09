use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::{
    card::Card,
    deck::Deck,
    depth::Depth,
    sources::Manifest,
    store::{ExamStamp, Store},
    token,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExamTopic {
    pub topic_id: String,
    pub label: String,
    pub context: Vec<String>,
    pub card_ids: Vec<String>,
    pub cards: Vec<String>,
    pub size: usize,
    pub sourced: bool,
    pub fingerprint: String,
}

pub const TOPIC_NUDGE_CARDS: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopicStatus {
    Unexamined,
    NeedsWork,
    Passed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TopicTally {
    pub passed: usize,
    pub needs_work: usize,
    pub unexamined: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExamFilter {
    #[default]
    None,
    Partial,
    All,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExamStatus {
    pub topics: Vec<TopicStatus>,
}

impl ExamStatus {
    pub fn all_passed(&self) -> bool {
        self.topics.iter().all(|s| *s == TopicStatus::Passed)
    }

    pub fn tally(&self) -> TopicTally {
        let mut tally = TopicTally::default();
        for status in &self.topics {
            match status {
                TopicStatus::Passed => tally.passed += 1,
                TopicStatus::NeedsWork => tally.needs_work += 1,
                TopicStatus::Unexamined => tally.unexamined += 1,
            }
        }
        tally
    }

    pub fn filter(&self) -> ExamFilter {
        let selected = self
            .topics
            .iter()
            .filter(|s| **s != TopicStatus::Passed)
            .count();
        if selected == 0 {
            ExamFilter::None
        } else if selected == self.topics.len() {
            ExamFilter::All
        } else {
            ExamFilter::Partial
        }
    }
}

impl ExamFilter {
    pub fn label(self) -> &'static str {
        match self {
            ExamFilter::None => "none",
            ExamFilter::Partial => "partial",
            ExamFilter::All => "all",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExamRow {
    pub passed_at_ms: Option<u64>,
    pub mastered_at_ms: Option<u64>,
    pub full_exam_result: Option<crate::store::FullExamResult>,
    pub tally: TopicTally,
    pub filter: ExamFilter,
    pub exam_changed: bool,
}

impl ExamRow {
    pub fn badge(&self, now_ms: u64) -> Option<String> {
        let today = crate::time::local_date(now_ms);
        let day = |at: u64| day_label(crate::time::local_date(at), today);
        let standing = standing_text(self.mastered_at_ms, self.passed_at_ms, now_ms);
        let last = self.full_exam_result.map(|result| {
            format!(
                "last ({}) {} / {}",
                day(result.at_ms),
                points(result.pass, result.partial),
                result.total
            )
        });
        match (standing, last) {
            (Some(standing), Some(last)) => Some(format!("{standing} · {last}")),
            (standing, last) => standing.or(last),
        }
    }
}

pub fn exam_high_mark(store: &Store, deck_id: &str, now_ms: u64) -> Option<String> {
    standing_text(
        store.deck_mastered_at(deck_id),
        store.deck_passed_at(deck_id),
        now_ms,
    )
}

fn standing_text(mastered_at: Option<u64>, passed_at: Option<u64>, now_ms: u64) -> Option<String> {
    let today = crate::time::local_date(now_ms);
    let day = |at: u64| day_label(crate::time::local_date(at), today);
    match (mastered_at, passed_at) {
        (Some(at), _) => Some(format!("mastered ({})", day(at))),
        (None, Some(at)) => Some(format!("passed ({})", day(at))),
        (None, None) => None,
    }
}

fn day_label(date: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    if date.year() == today.year() {
        date.format("%-d %b").to_string()
    } else {
        date.format("%-d %b %Y").to_string()
    }
}

fn points(pass: u32, partial: u32) -> String {
    let doubled = 2 * pass + partial;
    if doubled.is_multiple_of(2) {
        (doubled / 2).to_string()
    } else {
        format!("{}.5", doubled / 2)
    }
}

/// `personal` is the deck's bound sidecar cards when the caller already
/// holds them; `None` reads them only for a deck with a standing.
pub fn exam_row(deck: &Deck, store: &Store, personal: Option<&[Card]>) -> ExamRow {
    let deck_id = deck.deck_token.as_deref().unwrap_or_default();
    let passed_at_ms = store.deck_passed_at(deck_id);
    let mastered_at_ms = store.deck_mastered_at(deck_id);
    let standing = passed_at_ms.is_some() || mastered_at_ms.is_some();
    if !standing && !deck.declares_sources() {
        return ExamRow::default();
    }
    let status = exam_status(deck, store);
    let exam_changed = standing_stamp(store, deck_id).is_some() && {
        let read;
        let personal = match personal {
            Some(cards) => cards,
            None => {
                read = crate::assemble::personal_cards(deck).cards;
                &read
            }
        };
        standing_changed(
            store,
            deck_id,
            &current_stamp(deck, personal, manifest_of(deck)),
        )
    };
    ExamRow {
        passed_at_ms,
        mastered_at_ms,
        full_exam_result: store.full_exam_result(deck_id),
        tally: status.tally(),
        filter: status.filter(),
        exam_changed,
    }
}

pub fn exam_status(deck: &Deck, store: &Store) -> ExamStatus {
    let deck_id = deck.deck_token.as_deref().unwrap_or_default();
    status_of(deck_id, &examined_topics(deck), store)
}

pub fn manifest_of(deck: &Deck) -> Option<&Manifest> {
    deck.declared()
        .manifest
        .as_ref()
        .ok()
        .and_then(Option::as_ref)
}

/// The deck's sourced topics, as its sources manifest declares them.
pub fn examined_topics(deck: &Deck) -> Vec<ExamTopic> {
    exam_topics(deck, manifest_of(deck))
        .into_iter()
        .filter(|topic| topic.sourced)
        .collect()
}

pub fn status_of(deck_id: &str, topics: &[ExamTopic], store: &Store) -> ExamStatus {
    let topics = topics
        .iter()
        .map(|topic| topic_status(deck_id, topic, store))
        .collect();
    ExamStatus { topics }
}

fn topic_status(deck_id: &str, topic: &ExamTopic, store: &Store) -> TopicStatus {
    match store.topic_result(deck_id, &topic.topic_id) {
        Some(result) if result.fingerprint == topic.fingerprint => {
            if result.pass {
                TopicStatus::Passed
            } else {
                TopicStatus::NeedsWork
            }
        }
        _ => TopicStatus::Unexamined,
    }
}

pub fn oversized_topics(deck: &Deck) -> Vec<(String, usize)> {
    examined_topics(deck)
        .into_iter()
        .filter(|topic| topic.size > TOPIC_NUDGE_CARDS)
        .map(|topic| (topic.label, topic.size))
        .collect()
}

fn fingerprints_of(manifest: Option<&Manifest>) -> BTreeMap<String, String> {
    manifest
        .and_then(|manifest| manifest.fingerprints().ok())
        .unwrap_or_default()
}

/// One entry per topic with an id, in deck order, never merged: the leading
/// topic (the deck id) first and always, with or without cards.
pub fn exam_topics(deck: &Deck, sources: Option<&Manifest>) -> Vec<ExamTopic> {
    let sourced: BTreeSet<String> = sources.map(Manifest::sourced_topics).unwrap_or_default();
    let fingerprints = fingerprints_of(sources);
    let deck_id = deck.deck_token.as_deref().unwrap_or_default();
    let mut topics: Vec<ExamTopic> = Vec::with_capacity(deck.topics.len().max(1));
    let mut slots: Vec<Option<usize>> = Vec::with_capacity(deck.topics.len());
    for topic in &deck.topics {
        let Some(topic_id) = topic.id(deck_id) else {
            slots.push(None);
            continue;
        };
        let label = topic.heading.clone().unwrap_or_else(|| deck.display_name());
        let context = topic
            .heading
            .iter()
            .cloned()
            .chain(topic.prose.iter().cloned())
            .collect();
        slots.push(Some(topics.len()));
        topics.push(ExamTopic {
            sourced: sourced.contains(&topic_id),
            fingerprint: fingerprints.get(&topic_id).cloned().unwrap_or_default(),
            topic_id,
            label,
            context,
            ..Default::default()
        });
    }
    for card in &deck.cards {
        let Some(at) = slots.get(card.topic).copied().flatten() else {
            continue;
        };
        let topic = &mut topics[at];
        topic.size += 1;
        if let Some(id) = card.id() {
            topic.card_ids.push(id);
        }
        let scope = format!(
            "Q: {}\nA: {}",
            card.front.trim(),
            card.back.join("\n").trim()
        );
        if !topic.cards.contains(&scope) {
            topic.cards.push(scope);
        }
    }
    topics
}

/// Labels, in deck order, of the topics holding cards (the deck's and the
/// bound sidecar's) that no source binds.
pub fn unsourced_topic_labels(deck: &Deck, personal: &[Card]) -> Vec<String> {
    let (sourced_by_index, _) = topic_sourcing(deck, manifest_of(deck));
    let deck_id = deck.deck_token.as_deref().unwrap_or_default();
    let mut holds_cards = vec![false; deck.topics.len()];
    for card in deck.cards.iter().chain(personal) {
        if let Some(slot) = holds_cards.get_mut(card.topic) {
            *slot = true;
        }
    }
    deck.topics
        .iter()
        .enumerate()
        .filter(|(at, topic)| {
            topic.id(deck_id).is_some() && !sourced_by_index[*at] && holds_cards[*at]
        })
        .map(|(_, topic)| topic.heading.clone().unwrap_or_else(|| deck.display_name()))
        .collect()
}

pub fn sourced_hash(topics: &[(&str, &str)]) -> String {
    let mut lines: Vec<String> = topics
        .iter()
        .map(|(topic_id, fingerprint)| format!("{topic_id} {fingerprint}\n"))
        .collect();
    lines.sort_unstable();
    let mut hasher = Sha256::new();
    for line in &lines {
        hasher.update(line.as_bytes());
    }
    let digest: [u8; 32] = hasher.finalize().into();
    token::base32_256(&digest)
}

/// Per deck topic, in order, whether it has sources; and each sourced
/// topic's id with its fingerprint.
fn topic_sourcing(deck: &Deck, manifest: Option<&Manifest>) -> (Vec<bool>, Vec<(String, String)>) {
    let sourced: BTreeSet<String> = manifest.map(Manifest::sourced_topics).unwrap_or_default();
    let fingerprints = fingerprints_of(manifest);
    let deck_id = deck.deck_token.as_deref().unwrap_or_default();
    let mut by_index = Vec::with_capacity(deck.topics.len());
    let mut pairs = Vec::new();
    for topic in &deck.topics {
        let id = topic.id(deck_id).filter(|id| sourced.contains(id));
        by_index.push(id.is_some());
        if let Some(id) = id {
            let fingerprint = fingerprints.get(&id).cloned().unwrap_or_default();
            pairs.push((id, fingerprint));
        }
    }
    (by_index, pairs)
}

fn unsourced_card_ids(deck: &Deck, personal: &[Card], sourced_by_index: &[bool]) -> Vec<String> {
    let mut ids: Vec<String> = deck
        .cards
        .iter()
        .chain(personal)
        .filter(|card| !sourced_by_index.get(card.topic).copied().unwrap_or(false))
        .filter_map(Card::id)
        .collect();
    ids.sort_unstable();
    ids
}

fn stamp_of(pairs: &[(String, String)], unsourced_cards: Vec<String>) -> ExamStamp {
    let pairs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(id, fingerprint)| (id.as_str(), fingerprint.as_str()))
        .collect();
    ExamStamp {
        sourced: sourced_hash(&pairs),
        unsourced_cards,
    }
}

pub fn current_stamp(deck: &Deck, personal: &[Card], manifest: Option<&Manifest>) -> ExamStamp {
    let (sourced_by_index, pairs) = topic_sourcing(deck, manifest);
    stamp_of(
        &pairs,
        unsourced_card_ids(deck, personal, &sourced_by_index),
    )
}

fn standing_stamp<'a>(store: &'a Store, deck_id: &str) -> Option<&'a ExamStamp> {
    let has_standing =
        store.deck_passed_at(deck_id).is_some() || store.deck_mastered_at(deck_id).is_some();
    store.exam_stamp(deck_id).filter(|_| has_standing)
}

pub fn standing_changed(store: &Store, deck_id: &str, current: &ExamStamp) -> bool {
    standing_stamp(store, deck_id).is_some_and(|recorded| exam_changed(current, recorded))
}

pub fn exam_changed(current: &ExamStamp, recorded: &ExamStamp) -> bool {
    if current.sourced != recorded.sourced {
        return true;
    }
    let recorded: BTreeSet<&str> = recorded
        .unsourced_cards
        .iter()
        .map(String::as_str)
        .collect();
    current
        .unsourced_cards
        .iter()
        .any(|id| !recorded.contains(id.as_str()))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewCondition {
    pub holds: bool,
    pub blocking: Vec<String>,
}

pub fn review_condition(
    deck: &Deck,
    personal: &[Card],
    manifest: Option<&Manifest>,
    store: &Store,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> ReviewCondition {
    let (sourced_by_index, _) = topic_sourcing(deck, manifest);
    let unsourced = unsourced_card_ids(deck, personal, &sourced_by_index);
    let blocking = blocking_cards(&unsourced, store, now_ms, retire_after_days);
    ReviewCondition {
        holds: blocking.is_empty(),
        blocking,
    }
}

fn blocking_cards(
    unsourced: &[String],
    store: &Store,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> Vec<String> {
    unsourced
        .iter()
        .filter(|id| !meets_review_condition(id, store, now_ms, retire_after_days))
        .cloned()
        .collect()
}

fn meets_review_condition(
    card_id: &str,
    store: &Store,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> bool {
    if crate::session::is_retired_id(card_id, store, retire_after_days) {
        return true;
    }
    let Some(state) = store.get(card_id) else {
        return false;
    };
    let (depth, schedule) = match (&state.reconstruct, &state.recall) {
        (Some(schedule), _) => (Depth::Reconstruct, schedule),
        (None, Some(schedule)) => (Depth::Recall, schedule),
        (None, None) => return false,
    };
    let latest_passed = state
        .history
        .iter()
        .rev()
        .find(|review| review.depth == depth)
        .is_some_and(|review| review.grade.passed());
    schedule.graduated() && latest_passed && schedule.due_ms > now_ms
}

pub fn retire_after_days_for(
    review: crate::config::ReviewConfig,
    deck_path: &std::path::Path,
) -> Option<u32> {
    review
        .for_workspace(&crate::workspace::content_root(deck_path))
        .retire_after_days
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SittingStart {
    pub eligible: bool,
    pub blocking: Vec<String>,
    pub stamp: ExamStamp,
    pub unsourced_topics: Vec<String>,
}

/// Evaluated once per sitting, at `now_ms`, against the store the sitting
/// will be recorded in; the sidecar is read here.
pub fn sitting_start(
    deck: &Deck,
    store: &Store,
    now_ms: u64,
    retire_after_days: Option<u32>,
) -> SittingStart {
    let personal = crate::assemble::personal_cards(deck).cards;
    let (sourced_by_index, pairs) = topic_sourcing(deck, manifest_of(deck));
    let unsourced = unsourced_card_ids(deck, &personal, &sourced_by_index);
    let blocking = blocking_cards(&unsourced, store, now_ms, retire_after_days);
    SittingStart {
        eligible: blocking.is_empty(),
        blocking,
        stamp: stamp_of(&pairs, unsourced),
        unsourced_topics: unsourced_topic_labels(deck, &personal),
    }
}

pub enum ExamScope<'a> {
    Full,
    NeedsWork(&'a Store),
    Cards(&'a [String]),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExamSelection {
    pub topics: Vec<ExamTopic>,
    pub unknown_cards: Vec<String>,
}

pub fn select_topics(deck: &Deck, scope: ExamScope) -> ExamSelection {
    let topics = examined_topics(deck);
    match scope {
        ExamScope::Full => ExamSelection {
            topics,
            unknown_cards: Vec::new(),
        },
        ExamScope::NeedsWork(store) => {
            let deck_id = deck.deck_token.as_deref().unwrap_or_default();
            let status = status_of(deck_id, &topics, store);
            let selected: Vec<ExamTopic> = topics
                .iter()
                .zip(&status.topics)
                .filter(|(_, status)| **status != TopicStatus::Passed)
                .map(|(topic, _)| topic.clone())
                .collect();
            ExamSelection {
                topics: if selected.is_empty() {
                    topics
                } else {
                    selected
                },
                unknown_cards: Vec::new(),
            }
        }
        ExamScope::Cards(ids) => {
            let unknown_cards = ids
                .iter()
                .filter(|id| !topics.iter().any(|s| s.card_ids.contains(id)))
                .cloned()
                .collect();
            let selected = topics
                .into_iter()
                .filter(|topic| topic.card_ids.iter().any(|id| ids.contains(id)))
                .collect();
            ExamSelection {
                topics: selected,
                unknown_cards,
            }
        }
    }
}

pub fn needs_work_card_ids(deck: &Deck, store: &Store) -> Vec<String> {
    let deck_id = deck.deck_token.as_deref().unwrap_or_default();
    let topics = examined_topics(deck);
    let status = status_of(deck_id, &topics, store);
    topics
        .into_iter()
        .zip(status.topics)
        .filter(|(_, status)| *status != TopicStatus::Passed)
        .flat_map(|(topic, _)| topic.card_ids)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::{
        sources::{
            commands,
            testkit::{self, Workspace},
        },
        store::{
            CardState, ExamRecord, FsrsState, FullExamResult, Review, SittingRecord, TopicRecord,
        },
    };

    #[test]
    fn the_status_line_shows_the_standing_with_its_date_then_the_last_full_sitting() {
        const DAY_MS: u64 = 86_400_000;
        let now = 400 * DAY_MS;
        let (passed_at, mastered_at, sat_at) = (390 * DAY_MS, 398 * DAY_MS, 399 * DAY_MS);
        let today = crate::time::local_date(now);
        let day = |at: u64| day_label(crate::time::local_date(at), today);
        let full = |pass, partial, total| {
            Some(FullExamResult {
                at_ms: sat_at,
                pass,
                partial,
                total,
            })
        };
        let rows: [(&str, ExamRow, Option<String>); 6] = [
            ("never examined", ExamRow::default(), None),
            (
                "a full sitting without a standing",
                ExamRow {
                    full_exam_result: full(15, 0, 18),
                    ..ExamRow::default()
                },
                Some(format!("last ({}) 15 / 18", day(sat_at))),
            ),
            (
                "a partial is half a point",
                ExamRow {
                    full_exam_result: full(15, 1, 18),
                    ..ExamRow::default()
                },
                Some(format!("last ({}) 15.5 / 18", day(sat_at))),
            ),
            (
                "passed",
                ExamRow {
                    passed_at_ms: Some(passed_at),
                    full_exam_result: full(12, 0, 18),
                    tally: TopicTally {
                        needs_work: 2,
                        ..TopicTally::default()
                    },
                    ..ExamRow::default()
                },
                Some(format!(
                    "passed ({}) · last ({}) 12 / 18",
                    day(passed_at),
                    day(sat_at)
                )),
            ),
            (
                "passed through needs-work sittings only",
                ExamRow {
                    passed_at_ms: Some(passed_at),
                    ..ExamRow::default()
                },
                Some(format!("passed ({})", day(passed_at))),
            ),
            (
                "mastered wins over passed",
                ExamRow {
                    passed_at_ms: Some(passed_at),
                    mastered_at_ms: Some(mastered_at),
                    full_exam_result: full(18, 0, 18),
                    ..ExamRow::default()
                },
                Some(format!(
                    "mastered ({}) · last ({}) 18 / 18",
                    day(mastered_at),
                    day(sat_at)
                )),
            ),
        ];
        for (label, row, expected) in rows {
            assert_eq!(expected, row.badge(now), "{label}: {row:?}");
        }
    }

    #[test]
    fn a_status_date_names_the_year_only_outside_the_current_one() {
        let date = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let today = date(2026, 11, 5);
        for (label, day, expected) in [
            ("this year", date(2026, 11, 2), "2 Nov"),
            ("start of this year", date(2026, 1, 1), "1 Jan"),
            ("last year", date(2025, 12, 31), "31 Dec 2025"),
        ] {
            assert_eq!(expected, day_label(day, today), "{label}");
        }
    }

    fn pass_topics(store: &mut Store, deck: &Deck, topic_ids: &[&str], at_ms: u64) {
        let topics = examined_topics(deck);
        let record = SittingRecord {
            topics: topics
                .iter()
                .filter(|topic| topic_ids.contains(&topic.topic_id.as_str()))
                .map(|topic| TopicRecord {
                    topic_id: topic.topic_id.clone(),
                    fingerprint: topic.fingerprint.clone(),
                    pass: true,
                })
                .collect(),
            full: topic_ids.len() == topics.len(),
            pass: topic_ids.len() as u32,
            partial: 0,
            total: topic_ids.len() as u32,
        };
        let start = sitting_start(deck, store, at_ms, None);
        store.record_exam_result(deck, &record, at_ms, start.eligible, &start.stamp);
    }

    #[test]
    fn a_scope_selects_the_topics_it_names_and_reports_unknown_cards() {
        let (dir, decks) = testkit::workspace_decks();
        let path = decks.join("s.md");
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: deck-s\n---\n\
             # A\n\n## a1\nx\n<!-- id: card-a1 -->\n\n## a2\nx\n<!-- id: card-a2 -->\n\n\
             # B\n\n## b1\nx\n<!-- id: card-b1 -->\n",
        )
        .unwrap();
        let deck = Deck::load(testkit::sourced(path)).unwrap();
        let [a, b] = <[String; 2]>::try_from(
            examined_topics(&deck)
                .into_iter()
                .map(|topic| topic.topic_id)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let mut a_passed = Store::open(dir.path().join("a.json")).unwrap();
        pass_topics(&mut a_passed, &deck, &[&a], 1);
        let mut all_passed = a_passed.clone();
        pass_topics(&mut all_passed, &deck, &[&b], 2);
        let asked = ["card-b1".to_string(), "card-zz".to_string()];

        let rows = [
            ("full", ExamScope::Full, vec!["A", "B"], vec![]),
            (
                "needs work",
                ExamScope::NeedsWork(&a_passed),
                vec!["B"],
                vec![],
            ),
            (
                "needs work when every topic passed examines all",
                ExamScope::NeedsWork(&all_passed),
                vec!["A", "B"],
                vec![],
            ),
            (
                "named cards",
                ExamScope::Cards(&asked),
                vec!["B"],
                vec!["card-zz"],
            ),
        ];
        for (label, scope, expected, unknown) in rows {
            let selection = select_topics(&deck, scope);
            let labels: Vec<&str> = selection.topics.iter().map(|s| s.label.as_str()).collect();
            assert_eq!(expected, labels, "{label}: topics");
            assert_eq!(unknown, selection.unknown_cards, "{label}: unknown cards");
        }
        assert_eq!(
            vec!["card-b1".to_string()],
            needs_work_card_ids(&deck, &a_passed),
            "the phone sends the cards of the topics that need work"
        );
    }

    #[test]
    fn the_stamp_matches_the_frozen_vectors() {
        let a = "source-0mvea1nvj28mr91t2assfed9k3w5nrpbv6x05qyg7af1aq55s87m";
        let b = "source-0sfcvpgrsxk086zk7v55y3513c2m6tn8dw110m08s3pmx49rhb56";
        let fp_ab = crate::sources::topic_fingerprint(&[a, b]);
        let fp_a = crate::sources::topic_fingerprint(&[a]);
        assert_eq!(
            "1g2qyfy5b4bwnxgb09eeg9cychr7we5xx1zde0446a4sr4y16ed1",
            fp_ab
        );
        assert_eq!("11vgtp9hpsee7bsm548vqqntbvxpv13q300x04ktzq9nk8a7m5p3", fp_a);
        assert_eq!(
            "1bcw8rt11e2sqcfjfd1sg2t1a8dr0wcyk0ksw8f6e20j56bsc3p3",
            sourced_hash(&[
                ("topic-0000000000000000000000000a", fp_ab.as_str()),
                ("deck-0000000000000000000000000d", fp_a.as_str())
            ])
        );
        assert_eq!(
            "1rxgrh19hz0w2jdfqx68k5qvj917ns0y8s4vjd6a95cs3dw55e2n",
            sourced_hash(&[])
        );
    }

    #[test]
    fn an_unsorted_recorded_card_list_does_not_read_as_changed() {
        let ids = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        let recorded = ExamStamp {
            sourced: "s".to_string(),
            unsourced_cards: ids(&["card-c", "card-a", "card-b"]),
        };
        let current = ExamStamp {
            sourced: "s".to_string(),
            unsourced_cards: ids(&["card-a", "card-b", "card-c"]),
        };
        assert!(
            !exam_changed(&current, &recorded),
            "the same ids in another order are the same material"
        );
        let added = ExamStamp {
            sourced: "s".to_string(),
            unsourced_cards: ids(&["card-a", "card-b", "card-c", "card-d"]),
        };
        assert!(
            exam_changed(&added, &recorded),
            "control: a new id still dots"
        );
    }

    #[test]
    fn a_sitting_judges_retirement_by_the_workspace_overlay_on_the_review_config() {
        let (ws, _) = law_workspace();
        let deck = Deck::load(ws.deck()).unwrap();
        let mut store = Store::open(ws.root().join("p.json")).unwrap();
        let now = 1_000_000;
        for id in ["card-u2", "card-v1"] {
            *store.get_or_insert(id) = graduated_pass(now);
        }
        let mut rested = graduated_pass(now);
        if let Some(recall) = rested.recall.as_mut() {
            recall.scheduled_days = 20;
            recall.due_ms = now - 1;
        }
        *store.get_or_insert("card-u1") = rested;
        std::fs::write(
            ws.root().join(crate::config::LOCAL_MANIFEST),
            "[review]\nretire_after = \"10d\"\n",
        )
        .unwrap();

        let review = crate::config::ReviewConfig::default();
        let global = sitting_start(&deck, &store, now, review.retire_after_days);
        assert_eq!(
            vec!["card-u1".to_string()],
            global.blocking,
            "control: under the global cap the due card is not retired and blocks"
        );
        let overlay = retire_after_days_for(review, &ws.deck());
        assert_eq!(
            Some(10),
            overlay,
            "the workspace's own retire_after is the cap"
        );
        let local = sitting_start(&deck, &store, now, overlay);
        assert!(
            local.eligible,
            "retired under the workspace cap, so it meets the condition: {:?}",
            local.blocking
        );
    }

    const LAW_DECK: &str = "---\nformat-version: 1\nid: deck-fixture\n---\n\
        # A\n<!-- id: topic-a -->\n\n## a1\none\n<!-- id: card-a1 -->\n\n\
        # U\n<!-- id: topic-u -->\n\n## u1\ntwo\n<!-- id: card-u1 -->\n\n## u2\nthree\n<!-- id: card-u2 -->\n\n\
        # V\n<!-- id: topic-v -->\n\n## v1\nfour\n<!-- id: card-v1 -->\n";

    /// A workspace deck with sourced topic A (card a1) and unsourced topics U
    /// (u1, u2) and V (v1); the `a.txt` source is bound to A.
    fn law_workspace() -> (Workspace, String) {
        let ws = Workspace::new();
        std::fs::write(ws.deck(), LAW_DECK).unwrap();
        ws.write_live("a.txt", b"alpha source\n");
        let report = ws.add(&["a.txt"], &["topic-a"]).unwrap();
        let source_id = report.added[0].0.clone();
        (ws, source_id)
    }

    fn rewrite_deck(path: &Path, from: &str, to: &str) {
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains(from), "deck holds {from:?}: {text}");
        std::fs::write(path, text.replacen(from, to, 1)).unwrap();
    }

    fn sidecar_card(deck: &Path, id: &str, topic: &str) {
        let path = crate::personal::sidecar_path(deck);
        let mut text = std::fs::read_to_string(&path).unwrap_or_default();
        text.push_str(&format!(
            "## side {id}\nx\n<!-- topic: {topic} -->\n<!-- id: {id} -->\n\n"
        ));
        std::fs::write(path, text).unwrap();
    }

    fn graduated_pass(now_ms: u64) -> CardState {
        CardState {
            recall: Some(FsrsState {
                state: 2,
                scheduled_days: 5,
                due_ms: now_ms + 86_400_000,
                ..Default::default()
            }),
            history: vec![Review {
                ts_ms: now_ms.saturating_sub(1),
                grade: crate::scheduler::Grade::Pass,
                depth: Depth::Recall,
                propagated: false,
                confidence: None,
            }],
            ..CardState::new()
        }
    }

    fn row_after(ws: &Workspace, store: &Store) -> ExamRow {
        let deck = Deck::load(ws.deck()).unwrap();
        exam_row(&deck, store, None)
    }

    #[test]
    fn the_exam_screen_lists_unsourced_topics_by_label_with_the_deck_title_for_the_leading_topic() {
        let ws = Workspace::new();
        std::fs::write(
            ws.deck(),
            "---\nformat-version: 1\nid: deck-fixture\n---\n\
            ## lead\nzero\n<!-- id: card-l0 -->\n\n\
            # A\n<!-- id: topic-a -->\n\n## a1\none\n<!-- id: card-a1 -->\n\n\
            # Terms\n<!-- id: topic-terms -->\n\n## t1\ntwo\n<!-- id: card-t1 -->\n",
        )
        .unwrap();
        ws.write_live("a.txt", b"alpha source\n");
        ws.add(&["a.txt"], &["topic-a"]).unwrap();
        let deck = Deck::load(ws.deck()).unwrap();
        assert_eq!(
            vec![deck.display_name(), "Terms".to_string()],
            unsourced_topic_labels(&deck, &[]),
            "the leading topic is named by the deck title, never an empty heading"
        );
    }

    #[test]
    fn a_leading_topic_holding_only_a_sidecar_card_is_listed_unsourced() {
        let (ws, _) = law_workspace();
        sidecar_card(&ws.deck(), "card-side1", "deck-fixture");
        let deck = Deck::load(ws.deck()).unwrap();
        let store = Store::open(ws.root().join("p.json")).unwrap();
        let start = sitting_start(&deck, &store, 1_000_000, None);
        assert!(
            start
                .stamp
                .unsourced_cards
                .contains(&"card-side1".to_string()),
            "setup: the stamp counts the sidecar card as unsourced"
        );
        assert_eq!(
            vec![deck.display_name(), "U".to_string(), "V".to_string()],
            unsourced_topic_labels(&deck, &crate::assemble::personal_cards(&deck).cards),
            "a topic whose only cards are sidecar cards is left out too"
        );
    }

    #[test]
    fn a_standing_is_changed_for_a_sitting_stamp_only_when_the_recorded_stamp_differs() {
        let (ws, _) = law_workspace();
        let deck = Deck::load(ws.deck()).unwrap();
        let mut store = Store::open(ws.root().join("p.json")).unwrap();
        let now = 1_000_000;
        let stamp_of_deck = |deck: &Deck| {
            let personal = crate::assemble::personal_cards(deck).cards;
            current_stamp(deck, &personal, manifest_of(deck))
        };
        assert!(
            !standing_changed(&store, "deck-fixture", &stamp_of_deck(&deck)),
            "control: no standing, nothing changed"
        );
        for id in ["card-u1", "card-u2", "card-v1"] {
            *store.get_or_insert(id) = graduated_pass(now);
        }
        pass_topics(&mut store, &deck, &["topic-a"], now);
        assert!(
            !standing_changed(&store, "deck-fixture", &stamp_of_deck(&deck)),
            "the stamp the standing was earned on matches"
        );
        rewrite_deck(
            &ws.deck(),
            "## u2\nthree\n<!-- id: card-u2 -->\n",
            "## u2\nthree\n<!-- id: card-u2 -->\n\n## u3\nnew\n<!-- id: card-u3 -->\n",
        );
        let edited = Deck::load(ws.deck()).unwrap();
        assert!(
            standing_changed(&store, "deck-fixture", &stamp_of_deck(&edited)),
            "a card added to an unsourced topic changes the standing"
        );
    }

    #[test]
    fn exam_changed_is_set_exactly_on_the_ruled_changes() {
        type Mutate = Box<dyn Fn(&Workspace, &str, &mut Store)>;
        let deck_edit = |from: &'static str, to: &'static str| -> Mutate {
            Box::new(move |ws, _, _| rewrite_deck(&ws.deck(), from, to))
        };
        let rows: Vec<(&str, Mutate, bool)> = vec![
            (
                "sourced topic added",
                Box::new(|ws, _, _| {
                    ws.write_live("u.txt", b"u source\n");
                    ws.add(&["u.txt"], &["topic-u"]).unwrap();
                }),
                true,
            ),
            (
                "sourced topic removed (unbind all)",
                Box::new(|ws, source, _| {
                    commands::remove(&ws.deck(), &[source.to_string()]).unwrap();
                }),
                true,
            ),
            (
                "binding changed",
                Box::new(|ws, _, _| {
                    ws.rewrite_manifest("bound-to = [\"topic-a\"]", "bound-to = [\"1|topic-a\"]");
                }),
                true,
            ),
            (
                "source re-frozen (update)",
                Box::new(|ws, source, _| {
                    let file = ws.write_live("a.txt", b"alpha source, revised\n");
                    commands::update(
                        &ws.deck(),
                        source,
                        &file,
                        None,
                        false,
                        &crate::config::ExamConfig::default(),
                    )
                    .unwrap();
                }),
                true,
            ),
            (
                "card added to unsourced topic",
                deck_edit(
                    "## u2\nthree\n<!-- id: card-u2 -->\n",
                    "## u2\nthree\n<!-- id: card-u2 -->\n\n## u3\nnew\n<!-- id: card-u3 -->\n",
                ),
                true,
            ),
            (
                "new blank on an unsourced card",
                Box::new(|ws, _, _| {
                    rewrite_deck(
                        &ws.deck(),
                        "## u1\ntwo\n<!-- id: card-u1 -->\n",
                        "## u1\ntwo\n<!-- blank: span hidden=\"two\" -->\n<!-- id: card-u1 -->\n",
                    );
                    crate::stamp::stamp_deck(&ws.deck()).unwrap();
                }),
                true,
            ),
            (
                "card moved sourced -> unsourced",
                deck_edit(
                    "## a1\none\n<!-- id: card-a1 -->\n\n# U\n<!-- id: topic-u -->\n",
                    "# U\n<!-- id: topic-u -->\n\n## a1\none\n<!-- id: card-a1 -->\n",
                ),
                true,
            ),
            (
                "sidecar card added to unsourced topic",
                Box::new(|ws, _, _| sidecar_card(&ws.deck(), "card-side1", "topic-u")),
                true,
            ),
            (
                "card removed from unsourced topic",
                deck_edit("## u2\nthree\n<!-- id: card-u2 -->\n", ""),
                false,
            ),
            (
                "unsourced topic removed",
                deck_edit(
                    "# V\n<!-- id: topic-v -->\n\n## v1\nfour\n<!-- id: card-v1 -->\n",
                    "",
                ),
                false,
            ),
            (
                "card moved between unsourced topics",
                deck_edit(
                    "## v1\nfour\n<!-- id: card-v1 -->\n",
                    "## v1\nfour\n<!-- id: card-v1 -->\n\n## u2\nthree\n<!-- id: card-u2 -->\n",
                ),
                false,
            ),
            (
                "display path renamed (key or path)",
                Box::new(|ws, _, _| ws.rewrite_manifest("sdk:a.txt", "sdk:renamed.txt")),
                false,
            ),
            (
                "card text edited",
                deck_edit("## u1\ntwo\n", "## u1 (edited)\ntwo, revised\n"),
                false,
            ),
            ("heading edited", deck_edit("# U\n", "# U renamed\n"), false),
            (
                "card added to a sourced topic",
                deck_edit(
                    "## a1\none\n<!-- id: card-a1 -->\n",
                    "## a1\none\n<!-- id: card-a1 -->\n\n## a2\nmore\n<!-- id: card-a2 -->\n",
                ),
                false,
            ),
            (
                "card became due",
                Box::new(|_, _, store| {
                    store
                        .get_or_insert("card-u1")
                        .recall
                        .as_mut()
                        .unwrap()
                        .due_ms = 0;
                }),
                false,
            ),
            (
                "card lapsed",
                Box::new(|_, _, store| {
                    let state = store.get_or_insert("card-u1");
                    state.recall.as_mut().unwrap().state = 3;
                    state.record_review(50, crate::scheduler::Grade::Fail, Depth::Recall, false);
                }),
                false,
            ),
        ];
        let now = 1_000_000;
        for (label, mutate, expected) in rows {
            let (ws, source) = law_workspace();
            let deck = Deck::load(ws.deck()).unwrap();
            let mut store = Store::open(ws.root().join("p.json")).unwrap();
            for id in ["card-u1", "card-u2", "card-v1"] {
                *store.get_or_insert(id) = graduated_pass(now);
            }
            pass_topics(&mut store, &deck, &["topic-a"], now);
            assert!(
                store.deck_passed("deck-fixture"),
                "{label}: setup: the sitting passed the deck"
            );
            let card_moved = label == "card moved between unsourced topics";
            if card_moved {
                rewrite_deck(&ws.deck(), "## u2\nthree\n<!-- id: card-u2 -->\n\n", "");
            }
            mutate(&ws, &source, &mut store);
            let row = row_after(&ws, &store);
            assert_eq!(
                expected, row.exam_changed,
                "{label}: exam_changed after the change"
            );
            assert!(
                row.passed_at_ms.is_some(),
                "{label}: the standing stays latched"
            );
        }
    }

    #[test]
    fn an_exam_changed_passed_still_unlocks_a_dependent_and_an_exam_changed_mastered_stays_mastered()
     {
        let (ws, _) = law_workspace();
        std::fs::write(
            ws.root().join("decks").join("next.md"),
            "---\nformat-version: 1\nid: deck-next\nrequires: d\n---\n## n\n1\n<!-- id: card-n1 -->\n",
        )
        .unwrap();
        let next = testkit::sourced(ws.root().join("decks").join("next.md"));
        let deck = Deck::load(ws.deck()).unwrap();
        let mut store = Store::open(ws.root().join("p.json")).unwrap();
        let now = 1_000_000;
        for id in ["card-u1", "card-u2", "card-v1"] {
            *store.get_or_insert(id) = graduated_pass(now);
        }
        pass_topics(&mut store, &deck, &["topic-a"], now);
        assert!(
            store.deck_mastered("deck-fixture"),
            "setup: a full 100% sitting masters"
        );
        let next_deck = Deck::load(&next).unwrap();
        let decks_dir = ws.root().join("decks");
        assert!(
            !crate::deck::is_locked(&next_deck, Some(&decks_dir), &store),
            "setup: the dependent is unlocked by the pass"
        );

        rewrite_deck(
            &ws.deck(),
            "## u2\nthree\n<!-- id: card-u2 -->\n",
            "## u2\nthree\n<!-- id: card-u2 -->\n\n## u3\nnew\n<!-- id: card-u3 -->\n",
        );

        let row = row_after(&ws, &store);
        assert!(row.exam_changed, "a card added to an unsourced topic dots");
        assert_eq!(Some(now), row.mastered_at_ms, "mastered stays latched");
        assert!(
            !crate::deck::is_locked(&next_deck, Some(&decks_dir), &store),
            "a dotted standing never re-locks a dependent"
        );
    }

    #[test]
    fn the_review_condition_counts_recall_or_deeper_passes_propagated_credit_and_retired_cards_and_rejects_a_seen_press()
     {
        let now = 1_000_000;
        let cap = 30u32;
        let schedule = |state: u8, scheduled_days: u32, due_ms: u64| FsrsState {
            state,
            scheduled_days,
            due_ms,
            ..Default::default()
        };
        let review = |grade, depth, propagated| Review {
            ts_ms: now - 1,
            grade,
            depth,
            propagated,
            confidence: None,
        };
        use crate::scheduler::Grade::{Fail, Partial, Pass};
        let rows: Vec<(&str, CardState, bool)> = vec![
            ("never touched", CardState::new(), false),
            ("seen press only", CardState::introduced_at(now - 10), false),
            (
                "recall graduated, latest recall pass, not due",
                CardState {
                    recall: Some(schedule(2, 5, now + 1)),
                    history: vec![review(Pass, Depth::Recall, false)],
                    ..CardState::new()
                },
                true,
            ),
            (
                "recall graduated, latest recall a partial, not due",
                CardState {
                    recall: Some(schedule(2, 5, now + 1)),
                    history: vec![review(Partial, Depth::Recall, false)],
                    ..CardState::new()
                },
                true,
            ),
            (
                "recall graduated, latest recall entry propagated from a deeper pass",
                CardState {
                    recall: Some(schedule(2, 5, now + 1)),
                    history: vec![
                        review(Fail, Depth::Recall, false),
                        review(Pass, Depth::Recall, true),
                    ],
                    ..CardState::new()
                },
                true,
            ),
            (
                "recall graduated but due now",
                CardState {
                    recall: Some(schedule(2, 5, now)),
                    history: vec![review(Pass, Depth::Recall, false)],
                    ..CardState::new()
                },
                false,
            ),
            (
                "recall still learning",
                CardState {
                    recall: Some(schedule(1, 0, now + 1)),
                    history: vec![review(Pass, Depth::Recall, false)],
                    ..CardState::new()
                },
                false,
            ),
            (
                "recall graduated, latest recall failed",
                CardState {
                    recall: Some(schedule(3, 1, now + 1)),
                    history: vec![
                        review(Pass, Depth::Recall, false),
                        review(Fail, Depth::Recall, false),
                    ],
                    ..CardState::new()
                },
                false,
            ),
            (
                "recall graduated with a pass, only a later recognize fail",
                CardState {
                    recall: Some(schedule(2, 5, now + 1)),
                    recognize: Some(schedule(2, 5, now + 1)),
                    history: vec![
                        review(Pass, Depth::Recall, false),
                        review(Fail, Depth::Recognize, false),
                    ],
                    ..CardState::new()
                },
                true,
            ),
            (
                "recognize only",
                CardState {
                    recognize: Some(schedule(2, 50, now + 1)),
                    history: vec![review(Pass, Depth::Recognize, false)],
                    ..CardState::new()
                },
                false,
            ),
            (
                "reconstruct graduated and passed, recall failed",
                CardState {
                    recall: Some(schedule(3, 1, now)),
                    reconstruct: Some(schedule(2, 7, now + 1)),
                    history: vec![
                        review(Fail, Depth::Recall, false),
                        review(Pass, Depth::Reconstruct, false),
                    ],
                    ..CardState::new()
                },
                true,
            ),
            (
                "reconstruct still learning although recall is solid",
                CardState {
                    recall: Some(schedule(2, 20, now + 1)),
                    reconstruct: Some(schedule(1, 0, now + 1)),
                    history: vec![
                        review(Pass, Depth::Recall, false),
                        review(Pass, Depth::Reconstruct, false),
                    ],
                    ..CardState::new()
                },
                false,
            ),
            (
                "retired, even though due with a failed latest grade",
                CardState {
                    recall: Some(schedule(2, cap, now - 1)),
                    history: vec![review(Fail, Depth::Recall, false)],
                    ..CardState::new()
                },
                true,
            ),
        ];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("u.md");
        let mut text =
            String::from("---\nformat-version: 1\nid: deck-u\n---\n# U\n<!-- id: topic-u -->\n\n");
        for (at, _) in rows.iter().enumerate() {
            text.push_str(&format!("## q{at}\na\n<!-- id: card-r{at} -->\n\n"));
        }
        std::fs::write(&path, text).unwrap();
        let deck = Deck::load(&path).unwrap();
        let mut store = Store::open(dir.path().join("p.json")).unwrap();
        let mut expected_blocking = Vec::new();
        for (at, (label, state, satisfied)) in rows.iter().enumerate() {
            let id = format!("card-r{at}");
            *store.get_or_insert(&id) = state.clone();
            if !satisfied {
                expected_blocking.push((label, id));
            }
        }

        let condition = review_condition(&deck, &[], None, &store, now, Some(cap));

        let blocking: Vec<(&&str, String)> = rows
            .iter()
            .enumerate()
            .filter(|(at, _)| condition.blocking.contains(&format!("card-r{at}")))
            .map(|(at, (label, _, _))| (label, format!("card-r{at}")))
            .collect();
        assert_eq!(expected_blocking, blocking, "the blocking cards, by row");
        assert!(!condition.holds, "some row blocks");
        let all_satisfied: Vec<String> = rows
            .iter()
            .enumerate()
            .filter(|(_, (_, _, satisfied))| !*satisfied)
            .map(|(at, _)| format!("card-r{at}"))
            .collect();
        for id in &all_satisfied {
            *store.get_or_insert(id) = graduated_pass(now);
        }
        assert_eq!(
            ReviewCondition {
                holds: true,
                blocking: Vec::new()
            },
            review_condition(&deck, &[], None, &store, now, Some(cap)),
            "every card satisfied: the condition holds"
        );
    }

    #[test]
    fn an_ineligible_sitting_records_results_but_sets_no_standing() {
        let (ws, _) = law_workspace();
        let deck = Deck::load(ws.deck()).unwrap();
        let mut store = Store::open(ws.root().join("p.json")).unwrap();
        let now = 1_000_000;
        let start = sitting_start(&deck, &store, now, None);
        assert!(!start.eligible, "setup: the unsourced cards are unreviewed");
        assert_eq!(
            vec!["card-u1", "card-u2", "card-v1"],
            start.blocking,
            "setup: every unsourced card blocks"
        );
        let topics = examined_topics(&deck);
        let record = SittingRecord {
            topics: vec![TopicRecord {
                topic_id: topics[0].topic_id.clone(),
                fingerprint: topics[0].fingerprint.clone(),
                pass: true,
            }],
            full: true,
            pass: 1,
            partial: 0,
            total: 1,
        };

        let newly_passed =
            store.record_exam_result(&deck, &record, now, start.eligible, &start.stamp);

        assert!(!newly_passed, "an ineligible sitting unlocks nothing");
        assert_eq!(
            Some(true),
            store
                .topic_result("deck-fixture", "topic-a")
                .map(|result| result.pass),
            "the topic's result is recorded"
        );
        assert_eq!(
            vec![TopicStatus::Passed],
            exam_status(&deck, &store).topics,
            "the topic reads passed"
        );
        assert_eq!(
            None,
            store.deck_passed_at("deck-fixture"),
            "no passed instant"
        );
        assert_eq!(
            None,
            store.deck_mastered_at("deck-fixture"),
            "no mastered instant although the full sitting scored every point"
        );
        assert_eq!(None, store.exam_stamp("deck-fixture"), "no stamp");
        assert!(
            store.full_exam_result("deck-fixture").is_some(),
            "the full count is still kept"
        );
    }

    #[test]
    fn a_deck_with_no_sourced_topic_left_compares_against_the_zero_byte_hash() {
        let (ws, source) = law_workspace();
        let deck = Deck::load(ws.deck()).unwrap();
        let mut store = Store::open(ws.root().join("p.json")).unwrap();
        let now = 1_000_000;
        for id in ["card-u1", "card-u2", "card-v1"] {
            *store.get_or_insert(id) = graduated_pass(now);
        }
        pass_topics(&mut store, &deck, &["topic-a"], now);

        commands::remove(&ws.deck(), &[source]).unwrap();

        let deck = Deck::load(ws.deck()).unwrap();
        assert!(!deck.declares_sources(), "setup: no source is left");
        let personal = crate::assemble::personal_cards(&deck).cards;
        assert_eq!(
            "1rxgrh19hz0w2jdfqx68k5qvj917ns0y8s4vjd6a95cs3dw55e2n",
            current_stamp(&deck, &personal, manifest_of(&deck)).sourced,
            "zero sourced topics hash as zero bytes"
        );
        let row = exam_row(&deck, &store, Some(&personal));
        assert_eq!(Some(now), row.passed_at_ms, "the standing stays");
        assert!(row.exam_changed, "the recorded sourced hash differs");
    }

    #[test]
    fn a_new_sidecar_card_in_an_unsourced_topic_sets_exam_changed() {
        let (ws, _) = law_workspace();
        let deck = Deck::load(ws.deck()).unwrap();
        let mut store = Store::open(ws.root().join("p.json")).unwrap();
        let now = 1_000_000;
        for id in ["card-u1", "card-u2", "card-v1"] {
            *store.get_or_insert(id) = graduated_pass(now);
        }
        pass_topics(&mut store, &deck, &["topic-a"], now);
        assert!(!row_after(&ws, &store).exam_changed, "setup: solid");

        sidecar_card(&ws.deck(), "card-remedy1", "topic-u");

        assert!(
            row_after(&ws, &store).exam_changed,
            "a sidecar card bound to an unsourced topic dots the standing"
        );
    }

    #[test]
    fn a_sidecar_card_in_an_unsourced_topic_must_meet_the_review_condition() {
        let (ws, _) = law_workspace();
        let now = 1_000_000;
        let mut store = Store::open(ws.root().join("p.json")).unwrap();
        for id in ["card-u1", "card-u2", "card-v1"] {
            *store.get_or_insert(id) = graduated_pass(now);
        }
        sidecar_card(&ws.deck(), "card-remedy1", "topic-u");
        sidecar_card(&ws.deck(), "card-remedy2", "topic-a");
        let deck = Deck::load(ws.deck()).unwrap();

        let start = sitting_start(&deck, &store, now, None);

        assert!(!start.eligible, "an unreviewed sidecar card blocks");
        assert_eq!(
            vec!["card-remedy1".to_string()],
            start.blocking,
            "the sidecar card in the sourced topic does not count"
        );
        assert!(
            start
                .stamp
                .unsourced_cards
                .contains(&"card-remedy1".to_string()),
            "the stamp lists it: {:?}",
            start.stamp.unsourced_cards
        );
        *store.get_or_insert("card-remedy1") = graduated_pass(now);
        assert!(
            sitting_start(&deck, &store, now, None).eligible,
            "reviewed, the sidecar card satisfies the condition"
        );
    }

    #[test]
    fn a_skipped_sidecar_card_without_a_topic_stays_out_of_the_stamp_and_the_condition() {
        let (ws, _) = law_workspace();
        let now = 1_000_000;
        let mut store = Store::open(ws.root().join("p.json")).unwrap();
        for id in ["card-u1", "card-u2", "card-v1"] {
            *store.get_or_insert(id) = graduated_pass(now);
        }
        std::fs::write(
            crate::personal::sidecar_path(&ws.deck()),
            "## topicless\nx\n<!-- id: card-skip1 -->\n",
        )
        .unwrap();
        let deck = Deck::load(ws.deck()).unwrap();

        let start = sitting_start(&deck, &store, now, None);

        assert!(start.eligible, "a skipped card never blocks");
        assert!(
            !start
                .stamp
                .unsourced_cards
                .contains(&"card-skip1".to_string()),
            "a skipped card is in no topic: {:?}",
            start.stamp.unsourced_cards
        );
    }

    #[test]
    fn a_result_earned_on_older_material_does_not_count_and_a_needs_work_sitting_includes_the_topic()
     {
        let (ws, _) = law_workspace();
        let deck = Deck::load(ws.deck()).unwrap();
        let mut store = Store::open(ws.root().join("p.json")).unwrap();
        pass_topics(&mut store, &deck, &["topic-a"], 1);
        assert_eq!(
            vec![TopicStatus::Passed],
            exam_status(&deck, &store).topics,
            "setup: passed on the current material"
        );

        ws.rewrite_manifest("bound-to = [\"topic-a\"]", "bound-to = [\"1|topic-a\"]");

        let deck = Deck::load(ws.deck()).unwrap();
        assert_eq!(
            vec![TopicStatus::Unexamined],
            exam_status(&deck, &store).topics,
            "the result's fingerprint is no longer the topic's"
        );
        let selection = select_topics(&deck, ExamScope::NeedsWork(&store));
        assert_eq!(
            vec!["topic-a".to_string()],
            selection
                .topics
                .iter()
                .map(|topic| topic.topic_id.clone())
                .collect::<Vec<_>>(),
            "a needs-work sitting includes the changed topic"
        );
        assert_eq!(
            vec!["card-a1".to_string()],
            needs_work_card_ids(&deck, &store)
        );
        assert_eq!(
            Some(true),
            store
                .topic_result("deck-fixture", "topic-a")
                .map(|result| result.pass),
            "the stale result is kept, not deleted"
        );
    }

    fn cardless_sourced_workspace() -> Workspace {
        let ws = Workspace::new();
        std::fs::write(
            ws.deck(),
            "---\nformat-version: 1\nid: deck-fixture\n---\n# Empty\n<!-- id: topic-empty -->\n",
        )
        .unwrap();
        ws.write_live("e.txt", b"the whole topic\n");
        ws.add(&["e.txt"], &["topic-empty"]).unwrap();
        ws
    }

    #[test]
    fn a_sourced_topic_with_no_cards_passes_on_its_topic_result() {
        let ws = cardless_sourced_workspace();
        let deck = Deck::load(ws.deck()).unwrap();
        let topics = examined_topics(&deck);
        assert_eq!(1, topics.len(), "setup: the cardless topic is examined");
        assert!(topics[0].card_ids.is_empty(), "setup: it has no cards");
        let mut store = Store::open(ws.root().join("p.json")).unwrap();

        pass_topics(&mut store, &deck, &["topic-empty"], 7);

        assert_eq!(
            Some(&ExamRecord {
                at_ms: 7,
                pass: true,
                fingerprint: topics[0].fingerprint.clone(),
            }),
            store.topic_result("deck-fixture", "topic-empty"),
            "topic_results holds the result by topic id"
        );
        assert_eq!(vec![TopicStatus::Passed], exam_status(&deck, &store).topics);
        assert_eq!(Some(7), store.deck_passed_at("deck-fixture"));
    }

    #[test]
    fn adding_a_card_to_a_passed_topic_keeps_it_passed() {
        let (ws, _) = law_workspace();
        let deck = Deck::load(ws.deck()).unwrap();
        let mut store = Store::open(ws.root().join("p.json")).unwrap();
        pass_topics(&mut store, &deck, &["topic-a"], 1);

        rewrite_deck(
            &ws.deck(),
            "## a1\none\n<!-- id: card-a1 -->\n",
            "## a1\none\n<!-- id: card-a1 -->\n\n## a2\nmore\n<!-- id: card-a2 -->\n",
        );

        let deck = Deck::load(ws.deck()).unwrap();
        assert_eq!(
            vec![TopicStatus::Passed],
            exam_status(&deck, &store).topics,
            "the exam tests the sources, not the cards"
        );
        assert_eq!(
            TopicTally {
                passed: 1,
                ..TopicTally::default()
            },
            exam_row(&deck, &store, Some(&[])).tally
        );
    }

    #[test]
    fn a_result_for_a_topic_id_no_longer_in_the_deck_is_kept_and_ignored() {
        let (ws, _) = law_workspace();
        let deck = Deck::load(ws.deck()).unwrap();
        let path = ws.root().join("deck-fixture.json");
        let mut store = Store::open(&path).unwrap();
        let gone = SittingRecord {
            topics: vec![TopicRecord {
                topic_id: "topic-gone".to_string(),
                fingerprint: "f".to_string(),
                pass: false,
            }],
            ..Default::default()
        };
        store.record_exam_result(&deck, &gone, 3, false, &ExamStamp::default());
        store.save().unwrap();

        let store = Store::open(&path).unwrap();
        assert_eq!(
            Some(&ExamRecord {
                at_ms: 3,
                pass: false,
                fingerprint: "f".to_string(),
            }),
            store.topic_result("deck-fixture", "topic-gone"),
            "kept across a save"
        );
        assert_eq!(
            vec![TopicStatus::Unexamined],
            exam_status(&deck, &store).topics,
            "the deck's own topic is unaffected"
        );
        assert!(
            store.ever_examined(&deck),
            "a kept result still counts as examined"
        );
    }

    pub(crate) fn store_document(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn a_passed_deck_writes_its_stamp_and_results_into_the_document() {
        let (ws, _) = law_workspace();
        let deck = Deck::load(ws.deck()).unwrap();
        let path: PathBuf = ws.root().join("deck-fixture.json");
        let mut store = Store::open(&path).unwrap();
        let now = 1_000_000;
        for id in ["card-u1", "card-u2", "card-v1"] {
            *store.get_or_insert(id) = graduated_pass(now);
        }
        pass_topics(&mut store, &deck, &["topic-a"], now);
        store.save().unwrap();

        let document = store_document(&path);
        let progress = &document["deck"];
        assert_eq!(
            serde_json::json!(["card-u1", "card-u2", "card-v1"]),
            progress["exam_stamp"]["unsourced_cards"],
            "{document}"
        );
        assert_eq!(
            52,
            progress["exam_stamp"]["sourced"].as_str().unwrap().len()
        );
        assert_eq!(
            true, progress["topic_results"]["topic-a"]["pass"],
            "{document}"
        );
        let reopened = Store::open(&path).unwrap();
        assert_eq!(
            store.exam_stamp("deck-fixture"),
            reopened.exam_stamp("deck-fixture")
        );
    }
}
