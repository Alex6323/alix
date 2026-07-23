use std::{
    collections::HashSet,
    hash::{Hash, Hasher},
};

use crate::card::Card;

// One correct option plus three distractors.
pub const NUM_OPTIONS: usize = 4;

#[derive(Debug)]
pub struct ChoiceQuestion {
    pub options: Vec<String>,
    pub correct: usize,
}

fn answer_text(card: &Card) -> String {
    card.back.join("\n")
}

fn content(text: &str) -> String {
    crate::inline::strip_inline(text.trim())
}

// Seeded by appearance, not wall-clock: appearance only advances on a
// genuine re-serve, so this is stable across polls of one appearance.
pub fn seed_for(card_id: &str, appearance: u32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    card_id.hash(&mut hasher);
    appearance.hash(&mut hasher);
    hasher.finish()
}

fn distinct_distractors(card: &Card, ai_distractors: &[String]) -> Vec<String> {
    let needed = NUM_OPTIONS - 1;
    // Seed with the answer so no AI distractor can duplicate it.
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(content(&answer_text(card)));
    let mut chosen: Vec<String> = Vec::new();
    for option in ai_distractors {
        if chosen.len() == needed {
            break;
        }
        let trimmed = option.trim();
        let content = content(trimmed);
        if !content.is_empty() && seen.insert(content) {
            chosen.push(trimmed.to_string());
        }
    }
    chosen
}

pub fn build(card: &Card, seed: u64, ai_distractors: &[String]) -> Option<ChoiceQuestion> {
    let mut options = distinct_distractors(card, ai_distractors);
    if options.len() < NUM_OPTIONS - 1 {
        return None;
    }
    let correct_text = answer_text(card);
    options.push(correct_text.clone());
    let mut rng = Rng::new(seed);
    shuffle(&mut options, &mut rng);
    let correct = options.iter().position(|t| *t == correct_text)?;
    Some(ChoiceQuestion { options, correct })
}

pub fn build_authored(
    card: &Card,
    seed: u64,
    authored_distractors: &[String],
) -> Option<ChoiceQuestion> {
    let correct_text = answer_text(card);
    let mut seen = HashSet::new();
    seen.insert(content(&correct_text));
    let mut options = Vec::new();
    for distractor in authored_distractors {
        let trimmed = distractor.trim();
        let content = content(trimmed);
        if !content.is_empty() && seen.insert(content) {
            options.push(trimmed.to_string());
        }
    }
    if options.is_empty() {
        return None;
    }
    options.push(correct_text.clone());
    let mut rng = Rng::new(seed);
    shuffle(&mut options, &mut rng);
    let correct = options.iter().position(|option| *option == correct_text)?;
    Some(ChoiceQuestion { options, correct })
}

pub fn can_build(card: &Card, ai_distractors: &[String]) -> bool {
    distinct_distractors(card, ai_distractors).len() == NUM_OPTIONS - 1
}

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

// SplitMix64: good enough for shuffling options, and avoids a dependency.
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
    fn authored_build_uses_all_options_no_padding() {
        let mut c = card(1, "Paris");
        c.authored_distractors = vec!["London".into(), "Berlin".into()];
        let q = build_authored(&c, 1, &c.authored_distractors).unwrap();
        assert_eq!(3, q.options.len());
        assert_eq!("Paris", q.options[q.correct]);
        assert_eq!(
            1,
            q.options.iter().filter(|option| *option == "Paris").count()
        );
    }

    #[test]
    fn authored_build_needs_at_least_one_distractor() {
        let c = card(1, "Paris");
        assert!(build_authored(&c, 1, &[]).is_none());
    }

    #[test]
    fn fewer_than_three_distractors_yields_none() {
        let c = card(1, "alpha");
        assert!(build(&c, 42, &ai(&["beta", "gamma"])).is_none());
    }

    #[test]
    fn duplicate_distractors_count_once() {
        let c = card(1, "alpha");
        assert!(build(&c, 42, &ai(&["beta", "beta", "alpha"])).is_none());
    }

    #[test]
    fn ai_distractors_deduplicate_by_content_but_keep_source() {
        let c = card(1, "$x$");
        let q = build(&c, 42, &ai(&["x", "$y$", "z", "w"])).unwrap();
        assert_eq!("$x$", q.options[q.correct]);
        assert!(!q.options.iter().any(|option| option == "x"));
        assert!(q.options.iter().any(|option| option == "$y$"));
    }

    #[test]
    fn authored_distractors_deduplicate_by_content_but_keep_source() {
        let mut c = card(1, "$x^2$");
        c.authored_distractors = vec![
            "x^2".into(),
            "$x^3$".into(),
            "**four**".into(),
            "four".into(),
        ];
        let q = build_authored(&c, 1, &c.authored_distractors).unwrap();
        assert_eq!("$x^2$", q.options[q.correct]);
        assert!(!q.options.iter().any(|option| option == "x^2"));
        assert!(q.options.iter().any(|option| option == "$x^3$"));
        assert_eq!(1, q.options.iter().filter(|option| content(option) == "four").count());
    }

    #[test]
    fn an_ai_distractor_equal_to_the_answer_is_dropped() {
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
        let c = card(1, "alpha");
        assert!(recognition_question(&c, 1, Some(&ai(&["w1", "w2"]))).is_none());
        assert!(recognition_question(&c, 1, None).is_none());
    }

    #[test]
    fn recognition_question_rejects_multi_line_answers() {
        let c = card(1, "line a\nline b");
        let d = ai(&["w1", "w2", "w3"]);
        assert!(recognition_question(&c, 1, Some(&d)).is_none());
    }

    #[test]
    fn same_appearance_seed_is_stable_but_later_appearances_vary_the_order() {
        let c = card(1, "alpha");
        let d = ai(&["beta", "gamma", "delta"]);
        let id = "q42";

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
