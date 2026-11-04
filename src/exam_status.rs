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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExamFilter {
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
