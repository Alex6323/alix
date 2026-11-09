use crate::{deck::Deck, store::Store};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExamTopic {
    pub label: String,
    pub context: Vec<String>,
    pub card_ids: Vec<String>,
    pub cards: Vec<String>,
    pub size: usize,
}

pub const TOPIC_NUDGE_CARDS: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopicStatus {
    Unexamined,
    NeedsWork,
    NewCards,
    Passed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TopicTally {
    pub passed: usize,
    pub needs_work: usize,
    pub new_cards: usize,
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
                TopicStatus::NewCards => tally.new_cards += 1,
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

pub fn standing(store: &Store, deck_id: &str, now_ms: u64) -> Option<String> {
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

pub fn exam_row(deck: &Deck, store: &Store) -> ExamRow {
    if !deck.has_exam() {
        return ExamRow::default();
    }
    let deck_id = deck.deck_token.as_deref().unwrap_or_default();
    let status = exam_status(deck, store);
    ExamRow {
        passed_at_ms: store.deck_passed_at(deck_id),
        mastered_at_ms: store.deck_mastered_at(deck_id),
        full_exam_result: store.full_exam_result(deck_id),
        tally: status.tally(),
        filter: status.filter(),
    }
}

pub fn exam_status(deck: &Deck, store: &Store) -> ExamStatus {
    status_of(&exam_topics(deck), store)
}

pub fn status_of(topics: &[ExamTopic], store: &Store) -> ExamStatus {
    let topics = topics
        .iter()
        .map(|topic| topic_status(topic, store))
        .collect();
    ExamStatus { topics }
}

fn topic_status(topic: &ExamTopic, store: &Store) -> TopicStatus {
    let records: Vec<Option<bool>> = topic
        .card_ids
        .iter()
        .map(|id| {
            store
                .get(id)
                .and_then(|state| state.exam_result)
                .map(|r| r.pass)
        })
        .collect();
    if records.iter().all(Option::is_none) {
        TopicStatus::Unexamined
    } else if records.contains(&Some(false)) {
        TopicStatus::NeedsWork
    } else if records.contains(&None) {
        TopicStatus::NewCards
    } else {
        TopicStatus::Passed
    }
}

pub fn oversized_topics(deck: &Deck) -> Vec<(String, usize)> {
    exam_topics(deck)
        .into_iter()
        .filter(|topic| topic.size > TOPIC_NUDGE_CARDS)
        .map(|topic| (topic.label, topic.size))
        .collect()
}

pub fn exam_topics(deck: &Deck) -> Vec<ExamTopic> {
    let mut topics: Vec<ExamTopic> = Vec::new();
    for card in &deck.cards {
        let heading = card.topic_context.first();
        let at = match topics.iter().position(|s| s.context.first() == heading) {
            Some(at) => at,
            None => {
                topics.push(ExamTopic {
                    label: heading.cloned().unwrap_or_else(|| deck.display_name()),
                    context: card.topic_context.clone(),
                    ..Default::default()
                });
                topics.len() - 1
            }
        };
        topics[at].size += 1;
        if let Some(id) = card.id() {
            topics[at].card_ids.push(id);
        }
        let scope = format!(
            "Q: {}\nA: {}",
            card.front.trim(),
            card.back.join("\n").trim()
        );
        if !topics[at].cards.contains(&scope) {
            topics[at].cards.push(scope);
        }
    }
    topics
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
    let topics = exam_topics(deck);
    match scope {
        ExamScope::Full => ExamSelection {
            topics,
            unknown_cards: Vec::new(),
        },
        ExamScope::NeedsWork(store) => {
            let status = status_of(&topics, store);
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
    let topics = exam_topics(deck);
    let status = status_of(&topics, store);
    topics
        .into_iter()
        .zip(status.topics)
        .filter(|(_, status)| *status != TopicStatus::Passed)
        .flat_map(|(topic, _)| topic.card_ids)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ExamRecord, FullExamResult};

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

    #[test]
    fn a_scope_selects_the_topics_it_names_and_reports_unknown_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.md");
        std::fs::write(
            &path,
            "---\nformat-version: 1\nid: deck-s\n---\n\
             # A\n\n## a1\nx\n<!-- id: card-a1 -->\n\n## a2\nx\n<!-- id: card-a2 -->\n\n\
             # B\n\n## b1\nx\n<!-- id: card-b1 -->\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        let mut a_passed = Store::open(dir.path().join("a.json")).unwrap();
        for id in ["card-a1", "card-a2"] {
            a_passed.get_or_insert(id).exam_result = Some(ExamRecord {
                at_ms: 1,
                pass: true,
            });
        }
        let mut all_passed = a_passed.clone();
        all_passed.get_or_insert("card-b1").exam_result = Some(ExamRecord {
            at_ms: 1,
            pass: true,
        });
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
}
