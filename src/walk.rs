use std::collections::HashSet;

use crate::{
    answer::Mode,
    card::Card,
    choice::{self, ChoiceQuestion},
    review::{self, ChoiceFeedback, MultiChoiceFeedback},
    session,
    store::Store,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Front,
    Answer,
    Done,
}

pub struct WalkSession {
    cards: Vec<Card>,
    order: Vec<usize>,
    position: usize,
    phase: Phase,
    shown_sections: HashSet<Vec<String>>,
    section_first: bool,
    choice_seed: u64,
}

impl WalkSession {
    pub fn new(cards: Vec<Card>, store: &Store, now_ms: u64) -> WalkSession {
        let mut walk = WalkSession {
            cards,
            order: Vec::new(),
            position: 0,
            phase: Phase::Done,
            shown_sections: HashSet::new(),
            section_first: false,
            choice_seed: now_ms,
        };
        walk.restart(store, now_ms);
        walk
    }

    pub fn restart(&mut self, store: &Store, now_ms: u64) {
        self.order = walk_order(&self.cards, store);
        self.position = 0;
        self.shown_sections.clear();
        self.choice_seed = now_ms;
        self.enter();
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn total(&self) -> usize {
        self.order.len()
    }

    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    pub fn current(&self) -> Option<&Card> {
        self.order.get(self.position).map(|&i| &self.cards[i])
    }

    pub fn current_mut(&mut self) -> Option<&mut Card> {
        self.order.get(self.position).map(|&i| &mut self.cards[i])
    }

    pub fn section_first(&self) -> bool {
        self.section_first
    }

    pub fn question(&self) -> Option<ChoiceQuestion> {
        let card = self.current()?;
        let id = card.id()?;
        review::authored_question(card, choice::seed_for(&id, self.choice_seed, 0))
    }

    pub fn mode(&self) -> Mode {
        if self.question().is_some() {
            Mode::Choice
        } else {
            Mode::Flip
        }
    }

    pub fn choose(&mut self, chosen: usize) -> Option<ChoiceFeedback> {
        if self.phase != Phase::Front {
            return None;
        }
        let feedback = review::choice_feedback(&self.question()?, chosen)?;
        self.phase = Phase::Answer;
        Some(feedback)
    }

    pub fn choose_multi(&mut self, chosen: &[usize]) -> Option<MultiChoiceFeedback> {
        if self.phase != Phase::Front {
            return None;
        }
        let feedback = review::multi_choice_feedback(&self.question()?, chosen)?;
        self.phase = Phase::Answer;
        Some(feedback)
    }

    pub fn reveal(&mut self) {
        if self.phase == Phase::Front && self.question().is_none() {
            self.phase = Phase::Answer;
        }
    }

    /// The caller saves `store`.
    pub fn next(&mut self, store: &mut Store, now_ms: u64) {
        if self.phase != Phase::Answer {
            return;
        }
        if let Some(id) = self.current().and_then(Card::id) {
            store.get_or_insert(&id).walked_ms = Some(now_ms);
        }
        self.position += 1;
        self.enter();
    }

    fn enter(&mut self) {
        let Some(section) = self.current().map(|card| card.section_context.clone()) else {
            self.phase = Phase::Done;
            self.section_first = false;
            return;
        };
        self.phase = Phase::Front;
        self.section_first = !section.is_empty() && self.shown_sections.insert(section);
    }
}

fn walk_order(cards: &[Card], store: &Store) -> Vec<usize> {
    let mut order: Vec<usize> = (0..cards.len())
        .filter(|&i| cards[i].id().is_some())
        .collect();
    order.sort_by_cached_key(|&i| last_walked(&cards[i], store));
    let unwalked = order
        .iter()
        .take_while(|&&i| last_walked(&cards[i], store).is_none())
        .count();
    if unwalked == 0 {
        return order;
    }
    order.truncate(unwalked);
    let mut spaced = Vec::with_capacity(order.len());
    for run in order.chunk_by(|&a, &b| cards[a].section_context == cards[b].section_context) {
        let run = session::round_robin_siblings(run.to_vec(), cards);
        spaced.extend(session::separate_siblings(run, cards));
    }
    spaced
}

fn last_walked(card: &Card, store: &Store) -> Option<u64> {
    card.id()
        .and_then(|id| store.get(&id))
        .and_then(|state| state.walked_ms)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        depth::Depth,
        scheduler::{Fsrs, Grade, Scheduler},
        store::{CardState, FsrsState},
    };

    fn card(line: usize, section: &[&str]) -> Card {
        let mut card = Card::plain(
            Arc::from("deck.md"),
            format!("front {line}"),
            vec![format!("back {line}")],
            Vec::new(),
            line,
        );
        card.token = Some(Arc::from(format!("tok{line}").as_str()));
        card.section_context = section.iter().map(|text| (*text).to_string()).collect();
        card
    }

    fn reversed(card: &Card) -> Card {
        let mut reverse = card.clone();
        reverse.reversed = true;
        reverse
    }

    fn blank(card: &Card, stamp: &str) -> Card {
        let mut hole = card.clone();
        hole.region = Some(crate::card::RegionSlot::Single {
            stamp: Some(Arc::from(stamp)),
            hidden: card.back.first().cloned(),
            line: card.line,
            name: None,
        });
        hole
    }

    fn id(card: &Card) -> String {
        card.id().unwrap()
    }

    fn store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("deck-walk.json")).unwrap();
        (store, dir)
    }

    fn order_ids(walk: &WalkSession) -> Vec<String> {
        walk.order.iter().map(|&i| id(&walk.cards[i])).collect()
    }

    fn step(walk: &mut WalkSession, store: &mut Store, now_ms: u64) {
        if walk.mode() == Mode::Choice {
            let question = walk.question().unwrap();
            if question.multiple {
                walk.choose_multi(&question.correct_set);
            } else {
                walk.choose(question.correct);
            }
        } else {
            walk.reveal();
        }
        walk.next(store, now_ms);
    }

    #[test]
    fn law_a_walk_serves_only_unwalked_items_while_any_remain_else_every_item_oldest_first() {
        let cases: [[Option<u64>; 6]; 5] = [
            [None; 6],
            [Some(5), Some(4), Some(3), Some(2), Some(1), Some(0)],
            [Some(9), None, Some(3), None, Some(3), Some(1)],
            [Some(7), Some(7), Some(7), None, Some(7), None],
            [None, Some(2), None, Some(1), None, Some(2)],
        ];
        for (case, walked) in cases.iter().enumerate() {
            let cards: Vec<Card> = (0..walked.len()).map(|line| card(line, &[])).collect();
            let (mut store, _dir) = store();
            for (line, at) in walked.iter().enumerate() {
                if let Some(at) = at {
                    store.get_or_insert(&id(&cards[line])).walked_ms = Some(*at);
                }
            }
            let walk = WalkSession::new(cards, &store, 0);
            let unwalked: Vec<usize> = (0..walked.len()).filter(|&i| walked[i].is_none()).collect();
            let mut expected: Vec<usize> = if unwalked.is_empty() {
                (0..walked.len()).collect()
            } else {
                unwalked
            };
            expected.sort_by_key(|&i| (walked[i].unwrap_or(0), i));
            assert_eq!(
                (expected.len(), expected),
                (walk.total(), walk.order.clone()),
                "case {case} ({walked:?}): only the unwalked items while any remain, else all oldest first, ties in deck order"
            );
        }
    }

    #[test]
    fn law_a_walked_sibling_waits_while_unwalked_items_remain() {
        let section = ["# One"];
        let a = card(1, &section);
        let b = card(2, &section);
        let cards = vec![a.clone(), reversed(&a), b.clone()];
        let (mut store, _dir) = store();
        store.get_or_insert(&id(&b)).walked_ms = Some(5);
        let walk = WalkSession::new(cards, &store, 0);
        assert_eq!(
            vec![id(&a), id(&reversed(&a))],
            order_ids(&walk),
            "both never-walked directions of A are served, the walked B is not"
        );
    }

    #[test]
    fn law_a_walk_continues_where_the_previous_one_stopped() {
        let cards: Vec<Card> = (0..5).map(|line| card(line, &[])).collect();
        for stopped_after in 0..=cards.len() {
            let (mut store, _dir) = store();
            let mut walk = WalkSession::new(cards.clone(), &store, 0);
            let first = order_ids(&walk);
            for step_at in 0..stopped_after {
                step(&mut walk, &mut store, 100 + step_at as u64);
            }
            walk.restart(&store, 1_000);
            let expected = if stopped_after == cards.len() {
                first.clone()
            } else {
                first[stopped_after..].to_vec()
            };
            assert_eq!(
                expected,
                order_ids(&walk),
                "after stopping at {stopped_after}: the rest of the pass, or a fresh full pass once every item is walked"
            );
            assert_eq!(
                (0, Phase::Front),
                (walk.position(), walk.phase()),
                "after stopping at {stopped_after}: the next walk starts at its first item"
            );
        }
    }

    #[test]
    fn law_a_walk_is_one_pass_then_done() {
        let cards: Vec<Card> = (0..3).map(|line| card(line, &[])).collect();
        let (mut store, _dir) = store();
        let mut walk = WalkSession::new(cards, &store, 0);
        let mut served = Vec::new();
        for at in 0..walk.total() {
            served.push(id(walk.current().unwrap()));
            step(&mut walk, &mut store, 10 + at as u64);
        }
        assert_eq!(
            (Phase::Done, None),
            (walk.phase(), walk.current().map(id)),
            "after {served:?} the walk is done"
        );
        walk.reveal();
        walk.next(&mut store, 99);
        assert_eq!(
            (Phase::Done, 3),
            (walk.phase(), walk.position()),
            "reveal and next past the end change nothing"
        );
        let empty = WalkSession::new(Vec::new(), &store, 0);
        assert_eq!(
            (Phase::Done, 0),
            (empty.phase(), empty.total()),
            "an empty walk opens done"
        );
    }

    #[test]
    fn law_sibling_items_are_spread_within_their_section_and_sections_stay_contiguous() {
        let one = ["# One"];
        let two = ["# Two"];
        let a = card(1, &one);
        let b = card(2, &one);
        let c = card(3, &one);
        let d = card(4, &two);
        let e = card(5, &two);
        let cards = vec![
            a.clone(),
            reversed(&a),
            b,
            blank(&c, "c1"),
            blank(&c, "c2"),
            d.clone(),
            reversed(&d),
            e,
        ];
        let (store, _dir) = store();
        let walk = WalkSession::new(cards.clone(), &store, 0);
        let served: Vec<&Card> = walk.order.iter().map(|&i| &walk.cards[i]).collect();
        let mut sorted = order_ids(&walk);
        sorted.sort();
        let mut all: Vec<String> = cards.iter().map(id).collect();
        all.sort();
        assert_eq!(all, sorted, "every item is served exactly once");
        for pair in served.windows(2) {
            assert_ne!(
                pair[0].line,
                pair[1].line,
                "siblings {} and {} are adjacent in {:?}",
                id(pair[0]),
                id(pair[1]),
                order_ids(&walk)
            );
        }
        let sections: Vec<&Vec<String>> = served.iter().map(|card| &card.section_context).collect();
        let mut runs = sections.clone();
        runs.dedup();
        assert_eq!(
            2,
            runs.len(),
            "each section is one contiguous run: {sections:?}"
        );
    }

    #[test]
    fn law_section_first_marks_first_contact_in_every_walk() {
        let one = ["# One"];
        let two = ["# Two"];
        let cards = vec![
            card(1, &one),
            card(2, &one),
            card(3, &[]),
            card(4, &two),
            card(5, &two),
        ];
        let (mut store, _dir) = store();
        let mut walk = WalkSession::new(cards, &store, 0);
        let expected = vec![true, false, false, true, false];
        for round in 0..2 {
            let mut flags = Vec::new();
            let mut at = 100 * (round + 1);
            while walk.phase() != Phase::Done {
                flags.push(walk.section_first());
                step(&mut walk, &mut store, at);
                at += 1;
            }
            assert_eq!(
                expected, flags,
                "walk {round}: each section opens on its first item only"
            );
            walk.restart(&store, 1_000);
        }
        step(&mut walk, &mut store, 2_000);
        walk.restart(&store, 3_000);
        let mut flags = Vec::new();
        while walk.phase() != Phase::Done {
            flags.push(walk.section_first());
            step(&mut walk, &mut store, 4_000 + flags.len() as u64);
        }
        assert_eq!(
            2,
            flags.iter().filter(|&&first| first).count(),
            "a walk restarted mid-pass meets both sections afresh: {flags:?}"
        );
    }

    #[test]
    fn law_the_only_store_write_a_walk_makes_is_walked_ms_on_next() {
        let one = ["# One"];
        let picked = {
            let mut card = card(1, &one);
            card.back = vec!["right".to_string()];
            card.authored_distractors = vec!["wrong".to_string(), "also wrong".to_string()];
            card
        };
        let graded = card(2, &one);
        let introduced = card(3, &[]);
        let fresh = card(4, &[]);
        let cards = vec![
            picked.clone(),
            graded.clone(),
            reversed(&graded),
            introduced.clone(),
            fresh,
        ];
        let (mut store, _dir) = store();
        let scheduler = Fsrs::default();
        scheduler.apply(
            store.get_or_insert(&id(&graded)),
            Depth::Recall,
            Grade::Pass,
            50,
            false,
        );
        store.get_or_insert(&id(&introduced)).introduced_ms = Some(40);
        store.get_or_insert(&id(&picked)).recognize = Some(FsrsState {
            stability: 3.0,
            state: 2,
            ..Default::default()
        });
        store.set_last_depth("deck-walk", Depth::Recognize);
        store.set_deck_mastered("deck-walk", 60);
        let before = store.clone();
        let snapshot = |store: &Store| -> Vec<Option<CardState>> {
            cards
                .iter()
                .map(|card| store.get(&id(card)).cloned())
                .collect()
        };

        let mut walk = WalkSession::new(cards.clone(), &store, 0);
        let mut at = 1_000;
        while walk.phase() != Phase::Done {
            let current = id(walk.current().unwrap());
            let start = snapshot(&store);
            walk.next(&mut store, at);
            assert_eq!(
                start,
                snapshot(&store),
                "{current}: next on the front writes nothing"
            );
            if walk.mode() == Mode::Choice {
                walk.choose(0);
            } else {
                walk.reveal();
            }
            assert_eq!(
                (Phase::Answer, start),
                (walk.phase(), snapshot(&store)),
                "{current}: the attempt opens the answer and writes nothing"
            );
            walk.next(&mut store, at);
            assert_eq!(
                Some(at),
                store.get(&current).and_then(|state| state.walked_ms),
                "{current}: next past the answer stamps walked_ms"
            );
            at += 1;
        }
        for card in &cards {
            let card_id = id(card);
            let mut after = store.get(&card_id).cloned().unwrap();
            after.walked_ms = None;
            assert_eq!(
                before.get(&card_id).cloned().unwrap_or_default(),
                after,
                "{card_id}: everything but walked_ms is untouched"
            );
        }
        let absent = cards
            .iter()
            .filter(|card| before.get(&id(card)).is_none())
            .count();
        assert_eq!(
            (
                before.len() + absent,
                before.last_depth("deck-walk"),
                before.deck_mastered_at("deck-walk"),
                before.last_review_ms()
            ),
            (
                store.len(),
                store.last_depth("deck-walk"),
                store.deck_mastered_at("deck-walk"),
                store.last_review_ms()
            ),
            "deck progress is untouched and only never-touched items gain an entry"
        );
        for depth in [Depth::Recognize, Depth::Recall, Depth::Reconstruct] {
            assert_eq!(
                before.badge_earned("deck-walk", depth),
                store.badge_earned("deck-walk", depth),
                "{depth:?}: no badge is recorded"
            );
        }
    }

    #[test]
    fn law_authored_choice_items_are_picked_and_every_other_item_flips() {
        let text = "## pick one capital\n- [x] Paris\n- [ ] London\n- [ ] Berlin\n<!-- choices: single -->\n\n\
                    ## pick every even\n- [x] 2\n- [x] 4\n- [ ] 3\n<!-- choices: multiple -->\n\n\
                    ## plain question\nplain answer\n\n\
                    ## both ways\nthe other way\n<!-- direction: both -->\n\n\
                    ## cloze\none and two\n<!-- blank: span hidden=\"one\" b:a1b2c3 -->\n<!-- blank: span hidden=\"two\" b:d4e5f6 -->\n";
        let (mut store, dir) = store();
        let path = dir.path().join("choices.md");
        std::fs::write(&path, text).unwrap();
        crate::stamp::stamp_deck(&path).unwrap();
        let cards = crate::deck::Deck::load(&path).unwrap().cards;
        assert_eq!(
            7,
            cards.iter().filter_map(Card::id).count(),
            "the fixture expands its directions and blanks into stamped items"
        );
        let mut walk = WalkSession::new(cards, &store, 7);
        let mut at = 1;
        while walk.phase() != Phase::Done {
            let card = walk.current().unwrap().clone();
            let label = format!("{} ({})", card.front, id(&card));
            let authored = card.front.starts_with("pick");
            let question = walk.question();
            assert_eq!(
                (authored, if authored { Mode::Choice } else { Mode::Flip }),
                (question.is_some(), walk.mode()),
                "{label}: authored choices pick, everything else flips"
            );
            if let Some(question) = question {
                assert!(
                    question.options.len() >= 3,
                    "{label}: the authored options are offered: {:?}",
                    question.options
                );
                assert_eq!(
                    card.multiple_choice, question.multiple,
                    "{label}: a select-all card asks for every correct option"
                );
                walk.reveal();
                assert_eq!(
                    Phase::Front,
                    walk.phase(),
                    "{label}: a choice item opens only through a pick"
                );
                let passed = if question.multiple {
                    walk.choose_multi(&question.correct_set).map(|f| f.passed)
                } else {
                    walk.choose(question.correct).map(|f| f.passed)
                };
                assert_eq!(Some(true), passed, "{label}: the correct pick passes");
            } else {
                assert_eq!(
                    (None, None),
                    (
                        walk.choose(0).map(|f| f.passed),
                        walk.choose_multi(&[0]).map(|f| f.passed)
                    ),
                    "{label}: a flip item takes no pick"
                );
                walk.reveal();
            }
            assert_eq!(Phase::Answer, walk.phase(), "{label}: the answer is open");
            walk.next(&mut store, at);
            at += 1;
        }
    }
}
