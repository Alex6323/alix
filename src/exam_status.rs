use crate::{deck::Deck, store::Store};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExamSection {
    pub label: String,
    pub context: Vec<String>,
    pub card_ids: Vec<String>,
    pub cards: Vec<String>,
    pub size: usize,
}

pub const SECTION_NUDGE_CARDS: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionStatus {
    Unexamined,
    NeedsWork,
    NewCards,
    Passed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SectionTally {
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
    pub sections: Vec<SectionStatus>,
}

impl ExamStatus {
    pub fn all_passed(&self) -> bool {
        self.sections.iter().all(|s| *s == SectionStatus::Passed)
    }

    pub fn tally(&self) -> SectionTally {
        let mut tally = SectionTally::default();
        for status in &self.sections {
            match status {
                SectionStatus::Passed => tally.passed += 1,
                SectionStatus::NeedsWork => tally.needs_work += 1,
                SectionStatus::NewCards => tally.new_cards += 1,
                SectionStatus::Unexamined => tally.unexamined += 1,
            }
        }
        tally
    }

    pub fn filter(&self) -> ExamFilter {
        let selected = self
            .sections
            .iter()
            .filter(|s| **s != SectionStatus::Passed)
            .count();
        if selected == 0 {
            ExamFilter::None
        } else if selected == self.sections.len() {
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
    pub full_exam_result: Option<crate::store::FullExamResult>,
    pub tally: SectionTally,
    pub filter: ExamFilter,
}

pub fn exam_row(deck: &Deck, store: &Store) -> ExamRow {
    if !deck.has_exam() {
        return ExamRow::default();
    }
    let deck_id = deck.deck_token.as_deref().unwrap_or_default();
    let status = exam_status(deck, store);
    ExamRow {
        passed_at_ms: store.deck_passed_at(deck_id),
        full_exam_result: store.full_exam_result(deck_id),
        tally: status.tally(),
        filter: status.filter(),
    }
}

pub fn exam_status(deck: &Deck, store: &Store) -> ExamStatus {
    status_of(&exam_sections(deck), store)
}

pub fn status_of(sections: &[ExamSection], store: &Store) -> ExamStatus {
    let sections = sections
        .iter()
        .map(|section| section_status(section, store))
        .collect();
    ExamStatus { sections }
}

fn section_status(section: &ExamSection, store: &Store) -> SectionStatus {
    let records: Vec<Option<bool>> = section
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
        SectionStatus::Unexamined
    } else if records.contains(&Some(false)) {
        SectionStatus::NeedsWork
    } else if records.contains(&None) {
        SectionStatus::NewCards
    } else {
        SectionStatus::Passed
    }
}

pub fn oversized_sections(deck: &Deck) -> Vec<(String, usize)> {
    exam_sections(deck)
        .into_iter()
        .filter(|section| section.size > SECTION_NUDGE_CARDS)
        .map(|section| (section.label, section.size))
        .collect()
}

pub fn exam_sections(deck: &Deck) -> Vec<ExamSection> {
    let mut sections: Vec<ExamSection> = Vec::new();
    for card in &deck.cards {
        let heading = card.section_context.first();
        let at = match sections.iter().position(|s| s.context.first() == heading) {
            Some(at) => at,
            None => {
                sections.push(ExamSection {
                    label: heading.cloned().unwrap_or_else(|| deck.display_name()),
                    context: card.section_context.clone(),
                    ..Default::default()
                });
                sections.len() - 1
            }
        };
        sections[at].size += 1;
        if let Some(id) = card.id() {
            sections[at].card_ids.push(id);
        }
        let scope = format!(
            "Q: {}\nA: {}",
            card.front.trim(),
            card.back.join("\n").trim()
        );
        if !sections[at].cards.contains(&scope) {
            sections[at].cards.push(scope);
        }
    }
    sections
}

pub enum ExamScope<'a> {
    Full,
    NeedsWork(&'a Store),
    Cards(&'a [String]),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExamSelection {
    pub sections: Vec<ExamSection>,
    pub unknown_cards: Vec<String>,
}

pub fn select_sections(deck: &Deck, scope: ExamScope) -> ExamSelection {
    let sections = exam_sections(deck);
    match scope {
        ExamScope::Full => ExamSelection {
            sections,
            unknown_cards: Vec::new(),
        },
        ExamScope::NeedsWork(store) => {
            let status = status_of(&sections, store);
            let selected: Vec<ExamSection> = sections
                .iter()
                .zip(&status.sections)
                .filter(|(_, status)| **status != SectionStatus::Passed)
                .map(|(section, _)| section.clone())
                .collect();
            ExamSelection {
                sections: if selected.is_empty() {
                    sections
                } else {
                    selected
                },
                unknown_cards: Vec::new(),
            }
        }
        ExamScope::Cards(ids) => {
            let unknown_cards = ids
                .iter()
                .filter(|id| !sections.iter().any(|s| s.card_ids.contains(id)))
                .cloned()
                .collect();
            let selected = sections
                .into_iter()
                .filter(|section| section.card_ids.iter().any(|id| ids.contains(id)))
                .collect();
            ExamSelection {
                sections: selected,
                unknown_cards,
            }
        }
    }
}

pub fn needs_work_card_ids(deck: &Deck, store: &Store) -> Vec<String> {
    let sections = exam_sections(deck);
    let status = status_of(&sections, store);
    sections
        .into_iter()
        .zip(status.sections)
        .filter(|(_, status)| *status != SectionStatus::Passed)
        .flat_map(|(section, _)| section.card_ids)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ExamRecord;

    #[test]
    fn a_scope_selects_the_sections_it_names_and_reports_unknown_cards() {
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
                "needs work when every section passed examines all",
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
            let selection = select_sections(&deck, scope);
            let labels: Vec<&str> = selection
                .sections
                .iter()
                .map(|s| s.label.as_str())
                .collect();
            assert_eq!(expected, labels, "{label}: sections");
            assert_eq!(unknown, selection.unknown_cards, "{label}: unknown cards");
        }
        assert_eq!(
            vec!["card-b1".to_string()],
            needs_work_card_ids(&deck, &a_passed),
            "the phone sends the cards of the sections that need work"
        );
    }
}
