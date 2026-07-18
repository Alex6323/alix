//! Multiple-choice questions (`--mode choice` / the Recognize depth).
//!
//! The wrong options (distractors) come only from a card's cached AI
//! augmentation (`alix deck augment --target choices`). Distractors are never
//! sampled from other cards' answers: an offline-guessed multiple-choice reads
//! as junk (unrelated options that give the answer away, or near-duplicates),
//! which is worse than an honest reveal. A card without a full set of cached AI
//! distractors renders no pick, and the frontend falls back to a plain flip.

use std::{
    collections::HashSet,
    hash::{Hash, Hasher},
};

use crate::card::Card;

/// Total number of options shown (one correct + three distractors).
pub const NUM_OPTIONS: usize = 4;

/// A built multiple-choice question.
#[derive(Debug)]
pub struct ChoiceQuestion {
    /// The options in display order.
    pub options: Vec<String>,
    /// Index of the correct option.
    pub correct: usize,
}

/// The text of a card's answer as a single option.
fn answer_text(card: &Card) -> String {
    card.back.join("\n")
}

/// Combines a card id with how many times it has appeared this session
/// ([`crate::session::Session::appearance`]) into the shuffle seed for
/// [`build`]/[`recognition_question`]. The client polls `GET /api/state` every
/// ~3s while a card is on screen, and both that endpoint and `/api/choose`
/// rebuild the question from scratch (no server-side caching) — so the seed
/// must depend only on things that don't change while the card sits on
/// screen: the card id and the appearance count are exactly that, and the
/// appearance count only advances on a genuine re-serve (see
/// `Session::advance`), never on an idle poll. That makes it stable *within*
/// one appearance and different (barring rare permutation collisions) on the
/// *next* one — deliberately not wall-clock, which would drift mid-poll.
pub fn seed_for(card_id: u64, appearance: u32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    card_id.hash(&mut hasher);
    appearance.hash(&mut hasher);
    hasher.finish()
}

/// Assembles a multiple-choice question for `card` from its cached AI
/// distractors: dedups them against the correct answer and each other, and
/// returns `None` when fewer than [`NUM_OPTIONS`] `- 1` distinct, non-empty
/// distractors remain. Distractors are never sampled from other cards.
///
/// This is the Recognize-depth entry point: it makes no assumption about the
/// answer's shape, so a multi-line (`% reveal: line`) card is quizzed on its
/// whole joined sequence against the AI's alternate orderings. The acquire-bar
/// entry [`recognition_question`] adds the atomic-answer guard on top.
pub fn build(card: &Card, seed: u64, ai_distractors: &[String]) -> Option<ChoiceQuestion> {
    let correct_text = answer_text(card);
    let needed = NUM_OPTIONS - 1;
    let mut rng = Rng::new(seed);

    // The correct answer plus everything already chosen, so no AI option can
    // duplicate them.
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(correct_text.clone());

    let mut chosen: Vec<String> = Vec::new();
    for option in ai_distractors {
        if chosen.len() == needed {
            break;
        }
        let trimmed = option.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
            chosen.push(trimmed.to_string());
        }
    }

    if chosen.len() < needed {
        return None;
    }

    let mut options = chosen;
    options.push(correct_text.clone());
    shuffle(&mut options, &mut rng);
    let correct = options.iter().position(|t| *t == correct_text)?;

    Some(ChoiceQuestion { options, correct })
}

/// Builds an acquire-bar recognition question under the strict bar that makes a
/// first-encounter guess worth the trouble: an **atomic** card (a single-line
/// answer) whose deck supplies a full set of *AI* distractors (`alix deck
/// augment --target choices`). A card without a cached augmentation (or with a
/// multi-line answer) returns `None`, and the frontend shows a plain reveal
/// (flip) instead of a junk pick. A seen card in a Recognize session goes
/// through [`build`] instead, which drops the atomic guard.
pub fn recognition_question(
    card: &Card,
    seed: u64,
    ai_distractors: Option<&[String]>,
) -> Option<ChoiceQuestion> {
    if card.back.len() != 1 {
        return None;
    }
    let ai = ai_distractors.filter(|d| d.len() >= NUM_OPTIONS - 1)?;
    build(card, seed, ai)
}

/// A small SplitMix64 PRNG; good enough for shuffling options and avoids a
/// dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// Fisher-Yates shuffle.
fn shuffle<T>(items: &mut [T], rng: &mut Rng) {
    for i in (1..items.len()).rev() {
        items.swap(i, rng.below(i + 1));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn card(line: usize, back: &str) -> Card {
        Card::plain(
            Arc::from("deck.txt"),
            format!("front {line}"),
            back.split('\n').map(String::from).collect(),
            None,
            line,
        )
    }

    fn ai(distractors: &[&str]) -> Vec<String> {
        distractors.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn question_has_four_options_with_correct_exactly_once() {
        let c = card(1, "alpha");
        let d = ai(&["beta", "gamma", "delta"]);
        let q = build(&c, 42, &d).unwrap();
        assert_eq!(NUM_OPTIONS, q.options.len());
        assert_eq!(1, q.options.iter().filter(|o| *o == "alpha").count());
        assert_eq!("alpha", q.options[q.correct]);
    }

    #[test]
    fn fewer_than_three_distractors_yields_none() {
        // A full set is NUM_OPTIONS - 1 distinct distractors; two can't build a
        // question, and there is no offline pool to top it up.
        let c = card(1, "alpha");
        assert!(build(&c, 42, &ai(&["beta", "gamma"])).is_none());
    }

    #[test]
    fn duplicate_distractors_count_once() {
        // Duplicates of each other, or of the answer, must not pad the set.
        let c = card(1, "alpha");
        assert!(build(&c, 42, &ai(&["beta", "beta", "alpha"])).is_none());
    }

    #[test]
    fn an_ai_distractor_equal_to_the_answer_is_dropped() {
        // "alpha" is the correct answer and must never appear as a distractor;
        // the fourth option keeps the set full after it is dropped.
        let c = card(1, "alpha");
        let d = ai(&["alpha", "beta", "gamma", "delta"]);
        for seed in 0..10 {
            let q = build(&c, seed, &d).unwrap();
            assert_eq!(1, q.options.iter().filter(|o| *o == "alpha").count());
        }
    }

    #[test]
    fn same_seed_same_question() {
        let c = card(1, "alpha");
        let d = ai(&["beta", "gamma", "delta"]);
        let a = build(&c, 7, &d).unwrap();
        let b = build(&c, 7, &d).unwrap();
        assert_eq!(a.options, b.options);
        assert_eq!(a.correct, b.correct);
    }

    #[test]
    fn different_seeds_vary_the_options() {
        let c = card(1, "alpha");
        let d = ai(&["beta", "gamma", "delta"]);
        let orders: HashSet<Vec<String>> = (0..10)
            .map(|seed| build(&c, seed, &d).unwrap().options)
            .collect();
        assert!(orders.len() > 1, "options never varied across seeds");
    }

    #[test]
    fn multi_line_answers_become_one_option() {
        let c = card(1, "line a\nline b");
        let d = ai(&["x", "y", "z"]);
        let q = build(&c, 1, &d).unwrap();
        assert_eq!("line a\nline b", q.options[q.correct]);
    }

    #[test]
    fn recognition_question_needs_atomic_answer_and_full_ai_distractors() {
        let c = card(1, "alpha");
        let d = ai(&["w1", "w2", "w3"]);
        let q = recognition_question(&c, 1, Some(&d)).unwrap();
        assert_eq!(NUM_OPTIONS, q.options.len());
        assert_eq!("alpha", q.options[q.correct]);
    }

    #[test]
    fn recognition_question_rejects_too_few_ai_distractors() {
        // A short AI set (or none at all) falls back to recall-acquire (None); it
        // is never rescued by offline sampling, since a junk pick is worse than
        // an honest reveal.
        let c = card(1, "alpha");
        assert!(recognition_question(&c, 1, Some(&ai(&["w1", "w2"]))).is_none());
        assert!(recognition_question(&c, 1, None).is_none());
    }

    #[test]
    fn recognition_question_rejects_multi_line_answers() {
        // An open / multi-line answer can't be a meaningful pick-one.
        let c = card(1, "line a\nline b");
        let d = ai(&["w1", "w2", "w3"]);
        assert!(recognition_question(&c, 1, Some(&d)).is_none());
    }

    #[test]
    fn same_appearance_seed_is_stable_but_later_appearances_vary_the_order() {
        // The client polls `GET /api/state` every ~3s while a card is on screen;
        // `seed_for` must rebuild the identical question for repeated polls of
        // the SAME appearance, but a card served again after cycling out (a
        // later appearance) must eventually land on a different order — no more
        // solving a retry by position memory ({#reorder-mc-on-each-appearance}).
        let c = card(1, "alpha");
        let d = ai(&["beta", "gamma", "delta"]);
        let id = 42;

        let first = build(&c, seed_for(id, 1), &d).unwrap();
        let first_again = build(&c, seed_for(id, 1), &d).unwrap();
        assert_eq!(
            first.options, first_again.options,
            "the same appearance must not reshuffle mid-poll"
        );

        // Allow for the rare same-permutation collision across a couple of
        // seeds by checking a handful of later appearances.
        let later_orders_differ = (2..12)
            .map(|appearance| build(&c, seed_for(id, appearance), &d).unwrap())
            .any(|q| q.options != first.options);
        assert!(
            later_orders_differ,
            "no later appearance ever varied the order"
        );
    }
}
