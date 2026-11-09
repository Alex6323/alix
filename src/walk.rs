use std::collections::HashSet;

use crate::{
    answer::Mode,
    card::Card,
    choice::{self, ChoiceQuestion},
    depth::Reveal,
    review::{self, ChoiceFeedback, MultiChoiceFeedback},
    scheduler::Grade,
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
    shown_topics: HashSet<Vec<String>>,
    topic_first: bool,
    choice_seed: u64,
    pick: Option<bool>,
    hint_opened: bool,
    tutorial: bool,
}

impl WalkSession {
    pub fn new(cards: Vec<Card>, store: &Store, now_ms: u64) -> WalkSession {
        let mut walk = WalkSession {
            cards,
            order: Vec::new(),
            position: 0,
            phase: Phase::Done,
            shown_topics: HashSet::new(),
            topic_first: false,
            choice_seed: now_ms,
            pick: None,
            hint_opened: false,
            tutorial: false,
        };
        walk.restart(store, now_ms);
        walk
    }

    pub fn restart(&mut self, store: &Store, now_ms: u64) {
        self.order = walk_order(&self.cards, store);
        self.position = 0;
        self.shown_topics.clear();
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

    pub fn mark_tutorial(&mut self) {
        self.tutorial = true;
    }

    pub fn allows_personal_writes(&self) -> bool {
        !self.tutorial
    }

    pub fn topic_first(&self) -> bool {
        self.topic_first
    }

    pub fn question(&self) -> Option<ChoiceQuestion> {
        let card = self.current()?;
        let id = card.id()?;
        review::authored_question(card, choice::seed_for(&id, self.choice_seed, 0))
    }

    pub fn mode(&self) -> Mode {
        if self.question().is_some() {
            Mode::Choice
        } else if self.current().and_then(|card| card.reveal) == Some(Reveal::Line) {
            Mode::LineByLine
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
        self.pick = Some(feedback.passed);
        Some(feedback)
    }

    pub fn choose_multi(&mut self, chosen: &[usize]) -> Option<MultiChoiceFeedback> {
        if self.phase != Phase::Front {
            return None;
        }
        let feedback = review::multi_choice_feedback(&self.question()?, chosen)?;
        self.phase = Phase::Answer;
        self.pick = Some(feedback.passed);
        Some(feedback)
    }

    pub fn reveal(&mut self) {
        if self.phase == Phase::Front && self.question().is_none() {
            self.phase = Phase::Answer;
        }
    }

    pub fn pick(&self) -> Option<bool> {
        self.pick
    }

    pub fn open_hint(&mut self, card_id: &str) -> bool {
        let opens = self.phase == Phase::Front
            && self.current().is_some_and(|card| {
                !card.hints.is_empty() && card.id().as_deref() == Some(card_id)
            });
        self.hint_opened |= opens;
        opens
    }

    pub fn accepts(&self, grade: Grade) -> bool {
        self.phase == Phase::Answer
            && match self.pick {
                None => true,
                Some(true) => grade != Grade::Partial,
                Some(false) => grade == Grade::Fail,
            }
    }

    /// The caller saves `store`.
    pub fn grade(&mut self, store: &mut Store, grade: Grade, now_ms: u64) -> bool {
        if !self.accepts(grade) {
            return false;
        }
        let grade = if self.hint_opened && grade == Grade::Pass {
            Grade::Partial
        } else {
            grade
        };
        if let Some(id) = self.current().and_then(Card::id) {
            let state = store.get_or_insert(&id);
            state.walked_ms = Some(now_ms);
            state.walk_grade = Some(grade);
        }
        self.position += 1;
        self.enter();
        true
    }

    fn enter(&mut self) {
        self.pick = None;
        self.hint_opened = false;
        let Some(topic) = self.current().map(|card| card.topic_context.clone()) else {
            self.phase = Phase::Done;
            self.topic_first = false;
            return;
        };
        self.phase = Phase::Front;
        self.topic_first = !topic.is_empty() && self.shown_topics.insert(topic);
    }
}

fn walk_order(cards: &[Card], store: &Store) -> Vec<usize> {
    let mut order: Vec<usize> = (0..cards.len())
        .filter(|&i| cards[i].id().is_some())
        .collect();
    let waiting: Vec<usize> = order
        .iter()
        .copied()
        .filter(|&i| !got(&cards[i], store))
        .collect();
    if waiting.is_empty() {
        order.sort_by_cached_key(|&i| last_walked(&cards[i], store));
        return order;
    }
    order = waiting;
    let mut spaced = Vec::with_capacity(order.len());
    for run in order.chunk_by(|&a, &b| cards[a].topic_context == cards[b].topic_context) {
        let run = session::round_robin_siblings(run.to_vec(), cards);
        spaced.extend(session::separate_siblings(run, cards));
    }
    spaced
}

fn got(card: &Card, store: &Store) -> bool {
    card.id()
        .and_then(|id| store.get(&id))
        .is_some_and(|state| state.walk_grade == Some(Grade::Pass))
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

    fn card(line: usize, topic: &[&str]) -> Card {
        let mut card = Card::plain(
            Arc::from("deck.md"),
            format!("front {line}"),
            vec![format!("back {line}")],
            Vec::new(),
            line,
        );
        card.token = Some(Arc::from(format!("tok{line}").as_str()));
        card.topic_context = topic.iter().map(|text| (*text).to_string()).collect();
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
        walk.grade(store, Grade::Pass, now_ms);
    }

    #[test]
    fn a_walk_allows_personal_writes_until_marked_tutorial() {
        let (mut store, _dir) = store();
        let mut walk = WalkSession::new(vec![card(1, &[]), card(2, &[])], &store, 1);
        assert!(
            walk.allows_personal_writes(),
            "new: an ordinary walk allows personal writes"
        );
        walk.mark_tutorial();
        assert!(
            !walk.allows_personal_writes(),
            "marked: a tutorial walk refuses personal writes"
        );
        step(&mut walk, &mut store, 2);
        walk.restart(&store, 3);
        assert!(
            !walk.allows_personal_writes(),
            "restart: the next tutorial walk still refuses personal writes"
        );
    }

    #[test]
    fn law_a_walk_serves_waiting_items_in_deck_order_else_every_item_oldest_first() {
        use Grade::{Fail, Partial, Pass};
        let got = |at: u64| (Some(at), Some(Pass));
        let cases: [[(Option<u64>, Option<Grade>); 6]; 6] = [
            [(None, None); 6],
            [got(5), got(4), got(3), got(2), got(1), got(0)],
            [
                got(9),
                (None, None),
                (Some(3), Some(Fail)),
                got(1),
                (Some(3), Some(Partial)),
                got(2),
            ],
            [
                got(7),
                (Some(8), None),
                got(7),
                (None, None),
                got(7),
                got(1),
            ],
            [
                (Some(4), Some(Fail)),
                got(2),
                (None, None),
                got(1),
                (Some(1), Some(Partial)),
                got(2),
            ],
            [got(3), got(3), got(1), got(3), got(2), got(1)],
        ];
        for (case, states) in cases.iter().enumerate() {
            let cards: Vec<Card> = (0..states.len()).map(|line| card(line, &[])).collect();
            let (mut store, _dir) = store();
            for (line, (walked, grade)) in states.iter().enumerate() {
                if walked.is_some() || grade.is_some() {
                    let state = store.get_or_insert(&id(&cards[line]));
                    state.walked_ms = *walked;
                    state.walk_grade = *grade;
                }
            }
            let walk = WalkSession::new(cards, &store, 0);
            let waiting: Vec<usize> = (0..states.len())
                .filter(|&i| states[i].1 != Some(Pass))
                .collect();
            let expected: Vec<usize> = if waiting.is_empty() {
                let mut all: Vec<usize> = (0..states.len()).collect();
                all.sort_by_key(|&i| (states[i].0.unwrap_or(0), i));
                all
            } else {
                waiting
            };
            assert_eq!(
                (expected.len(), expected),
                (walk.total(), walk.order.clone()),
                "case {case} ({states:?}): the waiting items in deck order while any wait, else all oldest first, ties in deck order"
            );
        }
    }

    #[test]
    fn law_a_got_sibling_stays_out_while_waiting_items_remain() {
        let topic = ["# One"];
        let a = card(1, &topic);
        let b = card(2, &topic);
        let cards = vec![a.clone(), reversed(&a), b.clone()];
        let (mut store, _dir) = store();
        let state = store.get_or_insert(&id(&b));
        state.walked_ms = Some(5);
        state.walk_grade = Some(Grade::Pass);
        let walk = WalkSession::new(cards, &store, 0);
        assert_eq!(
            vec![id(&a), id(&reversed(&a))],
            order_ids(&walk),
            "both never-walked directions of A are served, the got B is not"
        );
    }

    #[test]
    fn law_a_resumed_full_pass_opens_the_topic_of_its_first_item() {
        let cards: Vec<Card> = (0..4)
            .map(|line| card(line, if line < 2 { &["# S1"] } else { &["# S2"] }))
            .collect();
        let (mut store, _dir) = store();
        let mut walk = WalkSession::new(cards.clone(), &store, 0);
        for at in 0..4 {
            step(&mut walk, &mut store, 10 + at);
        }
        walk.restart(&store, 100);
        step(&mut walk, &mut store, 200);
        walk.restart(&store, 300);
        assert_eq!(
            (id(&cards[1]), true),
            (id(walk.current().unwrap()), walk.topic_first()),
            "the resumed full pass starts at the least recently walked item and opens its topic"
        );
    }

    #[test]
    fn law_a_missed_item_is_not_served_again_in_the_same_walk() {
        let cards: Vec<Card> = (0..3).map(|line| card(line, &[])).collect();
        let (mut store, _dir) = store();
        let mut walk = WalkSession::new(cards, &store, 0);
        let mut served = Vec::new();
        while walk.phase() != Phase::Done {
            served.push(id(walk.current().unwrap()));
            walk.reveal();
            walk.grade(&mut store, Grade::Fail, 5);
        }
        let mut unique = served.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            (3, unique.len()),
            (served.len(), served.len()),
            "every missed item is served once: {served:?}"
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
        assert_eq!(
            (false, Phase::Done, 3),
            (
                walk.grade(&mut store, Grade::Pass, 99),
                walk.phase(),
                walk.position()
            ),
            "reveal and grade past the end change nothing"
        );
        let empty = WalkSession::new(Vec::new(), &store, 0);
        assert_eq!(
            (Phase::Done, 0),
            (empty.phase(), empty.total()),
            "an empty walk opens done"
        );
    }

    #[test]
    fn law_sibling_items_are_spread_within_their_topic_and_topics_stay_contiguous() {
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
        let topics: Vec<&Vec<String>> = served.iter().map(|card| &card.topic_context).collect();
        let mut runs = topics.clone();
        runs.dedup();
        assert_eq!(
            2,
            runs.len(),
            "each topic is one contiguous run: {topics:?}"
        );
    }

    #[test]
    fn law_topic_first_marks_first_contact_in_every_walk() {
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
                flags.push(walk.topic_first());
                step(&mut walk, &mut store, at);
                at += 1;
            }
            assert_eq!(
                expected, flags,
                "walk {round}: each topic opens on its first item only"
            );
            walk.restart(&store, 1_000);
        }
        step(&mut walk, &mut store, 2_000);
        walk.restart(&store, 3_000);
        let mut flags = Vec::new();
        while walk.phase() != Phase::Done {
            flags.push(walk.topic_first());
            step(&mut walk, &mut store, 4_000 + flags.len() as u64);
        }
        assert_eq!(
            2,
            flags.iter().filter(|&&first| first).count(),
            "a walk restarted mid-pass meets both topics afresh: {flags:?}"
        );
    }

    #[test]
    fn law_a_walk_writes_only_walked_ms_and_walk_grade_on_grade() {
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
        store.set_last_launch("deck-walk", crate::depth::Launch::Recognize);
        store.set_deck_passed("deck-walk", 60, &crate::store::ExamStamp::default());
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
            assert!(
                !walk.grade(&mut store, Grade::Pass, at),
                "{current}: grade on the front is refused"
            );
            assert_eq!(
                start,
                snapshot(&store),
                "{current}: grade on the front writes nothing"
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
            let grade = if walk.pick() == Some(false) {
                Grade::Fail
            } else {
                Grade::Pass
            };
            assert!(
                walk.grade(&mut store, grade, at),
                "{current}: {grade:?} after the attempt is accepted"
            );
            assert_eq!(
                (Some(at), Some(grade)),
                store
                    .get(&current)
                    .map(|state| (state.walked_ms, state.walk_grade))
                    .unwrap_or_default(),
                "{current}: grading stamps walked_ms and stores the grade"
            );
            at += 1;
        }
        for card in &cards {
            let card_id = id(card);
            let mut after = store.get(&card_id).cloned().unwrap();
            after.walked_ms = None;
            after.walk_grade = None;
            assert_eq!(
                before.get(&card_id).cloned().unwrap_or_default(),
                after,
                "{card_id}: everything but walked_ms and walk_grade is untouched"
            );
        }
        let absent = cards
            .iter()
            .filter(|card| before.get(&id(card)).is_none())
            .count();
        assert_eq!(
            (
                before.len() + absent,
                before.last_launch("deck-walk"),
                before.deck_passed_at("deck-walk"),
                before.last_review_ms()
            ),
            (
                store.len(),
                store.last_launch("deck-walk"),
                store.deck_passed_at("deck-walk"),
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

    fn items_by_kind() -> (Vec<(&'static str, Card)>, tempfile::TempDir) {
        let text = "## pick one capital\n- [x] Paris\n- [ ] London\n- [ ] Berlin\n<!-- choices: single -->\n\n\
                    ## pick every even\n- [x] 2\n- [x] 4\n- [ ] 3\n<!-- choices: multiple -->\n\n\
                    ## plain question\nplain answer\n\n\
                    ## lines question\nfirst line\nsecond line\n<!-- reveal: line -->\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kinds.md");
        std::fs::write(&path, text).unwrap();
        crate::stamp::stamp_deck(&path).unwrap();
        let cards = crate::deck::Deck::load(&path).unwrap().cards;
        let by_front = |front: &str| cards.iter().find(|c| c.front == front).unwrap().clone();
        (
            vec![
                ("single", by_front("pick one capital")),
                ("multi", by_front("pick every even")),
                ("flip", by_front("plain question")),
                ("line", by_front("lines question")),
            ],
            dir,
        )
    }

    fn attempt(walk: &mut WalkSession, right: bool) {
        match walk.question() {
            Some(question) if question.multiple => {
                let chosen: Vec<usize> = if right {
                    question.correct_set.clone()
                } else {
                    (0..question.options.len())
                        .filter(|i| !question.correct_set.contains(i))
                        .take(1)
                        .collect()
                };
                walk.choose_multi(&chosen);
            }
            Some(question) => {
                let chosen = if right {
                    question.correct
                } else {
                    (question.correct + 1) % question.options.len()
                };
                walk.choose(chosen);
            }
            None => walk.reveal(),
        }
    }

    #[test]
    fn law_a_walk_grade_follows_the_accept_table_for_every_item_kind() {
        let (items, _deck_dir) = items_by_kind();
        for (kind, item) in &items {
            let picks: &[bool] = if item.authored_distractors.is_empty() && !item.multiple_choice {
                &[true]
            } else {
                &[true, false]
            };
            for &right in picks {
                for grade in [Grade::Fail, Grade::Partial, Grade::Pass] {
                    for hinted in [false, true] {
                        let mut card = item.clone();
                        card.hints = vec!["a hint".to_string()];
                        let card_id = id(&card);
                        let (mut store, _dir) = store();
                        let mut walk = WalkSession::new(vec![card], &store, 3);
                        let picked = walk.mode() == Mode::Choice;
                        if hinted {
                            assert!(
                                walk.open_hint(&card_id),
                                "{kind}: the hint opens on the front"
                            );
                        }
                        attempt(&mut walk, right);
                        let label = format!("{kind} right={right} {grade:?} hinted={hinted}");
                        let accepted = match (picked, right) {
                            (false, _) => true,
                            (true, true) => grade != Grade::Partial,
                            (true, false) => grade == Grade::Fail,
                        };
                        assert_eq!(
                            (accepted, picked.then_some(right)),
                            (walk.accepts(grade), walk.pick()),
                            "{label}: accepts and pick"
                        );
                        let before = store.clone();
                        assert_eq!(
                            accepted,
                            walk.grade(&mut store, grade, 50),
                            "{label}: grade's verdict"
                        );
                        let stored = store.get(&card_id).map(|s| (s.walked_ms, s.walk_grade));
                        let expected = accepted.then(|| {
                            let capped = if hinted && grade == Grade::Pass {
                                Grade::Partial
                            } else {
                                grade
                            };
                            (Some(50), Some(capped))
                        });
                        assert_eq!(expected, stored, "{label}: what the store holds");
                        if !accepted {
                            assert_eq!(
                                (before.len(), Phase::Answer),
                                (store.len(), walk.phase()),
                                "{label}: a refused grade writes nothing and stays on the answer"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn law_open_hint_is_accepted_only_in_front_on_the_current_item_with_hints() {
        let mut hinted = card(1, &[]);
        hinted.hints = vec!["look left".to_string()];
        let bare = card(2, &[]);
        let (hinted_id, bare_id) = (id(&hinted), id(&bare));
        let (mut store, _dir) = store();
        let mut walk = WalkSession::new(vec![hinted, bare], &store, 0);
        assert_eq!(
            (false, true),
            (walk.open_hint(&bare_id), walk.open_hint(&hinted_id)),
            "front: another item's id is refused, the current one opens"
        );
        walk.reveal();
        assert!(!walk.open_hint(&hinted_id), "answer: refused");
        walk.grade(&mut store, Grade::Pass, 5);
        assert_eq!(
            Some(Grade::Partial),
            store.get(&hinted_id).and_then(|s| s.walk_grade),
            "the opened hint capped the pass"
        );
        assert!(
            !walk.open_hint(&bare_id),
            "front of an item without hints: refused"
        );
        walk.reveal();
        walk.grade(&mut store, Grade::Pass, 6);
        assert_eq!(
            Some(Grade::Pass),
            store.get(&bare_id).and_then(|s| s.walk_grade),
            "the cap does not carry over to the next item"
        );
    }

    #[test]
    fn law_authored_choice_items_are_picked_line_cards_reveal_by_line_and_the_rest_flip() {
        let text = "## pick one capital\n- [x] Paris\n- [ ] London\n- [ ] Berlin\n<!-- choices: single -->\n\n\
                    ## pick every even\n- [x] 2\n- [x] 4\n- [ ] 3\n<!-- choices: multiple -->\n\n\
                    ## plain question\nplain answer\n\n\
                    ## lines question\nfirst line\nsecond line\n<!-- reveal: line -->\n\n\
                    ## both ways\nthe other way\n<!-- direction: both -->\n\n\
                    ## cloze\none and two\n<!-- blank: span hidden=\"one\" b:a1b2c3 -->\n<!-- blank: span hidden=\"two\" b:d4e5f6 -->\n";
        let (mut store, dir) = store();
        let path = dir.path().join("choices.md");
        std::fs::write(&path, text).unwrap();
        crate::stamp::stamp_deck(&path).unwrap();
        let cards = crate::deck::Deck::load(&path).unwrap().cards;
        assert_eq!(
            8,
            cards.iter().filter_map(Card::id).count(),
            "the fixture expands its directions and blanks into stamped items"
        );
        let mut walk = WalkSession::new(cards, &store, 7);
        let mut at = 1;
        while walk.phase() != Phase::Done {
            let card = walk.current().unwrap().clone();
            let label = format!("{} ({})", card.front, id(&card));
            let authored = card.front.starts_with("pick");
            let expected = if authored {
                Mode::Choice
            } else if card.front.starts_with("lines") {
                Mode::LineByLine
            } else {
                Mode::Flip
            };
            let question = walk.question();
            assert_eq!(
                (authored, expected),
                (question.is_some(), walk.mode()),
                "{label}: authored choices pick, `reveal: line` reveals by line, everything else flips"
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
            walk.grade(&mut store, Grade::Fail, at);
            at += 1;
        }
    }
}
