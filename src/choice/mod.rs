pub mod sample;

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

// Stable across polls of one appearance, but a fresh study session receives a
// new session seed so a remembered option position does not carry between runs.
pub fn seed_for(card_id: &str, session_seed: u64, appearance: u32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    card_id.hash(&mut hasher);
    session_seed.hash(&mut hasher);
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

/// The card's same-deck sampling candidates: other base tokens only (a
/// cloze sibling or reversed half must never leak its answer), single-line
/// backs, deduplicated by plain content, the card's own answer excluded by
/// content so a markup variant cannot slip past the sampler's exact
/// equality.
pub fn sampled_pool(card: &Card, cards: &[Card]) -> Vec<String> {
    if card.deck_id.is_empty() {
        return Vec::new();
    }
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(content(&answer_text(card)));
    let mut pool = Vec::new();
    for other in cards {
        if other.deck_id != card.deck_id || other.token == card.token || other.back.len() != 1 {
            continue;
        }
        let text = answer_text(other).trim().to_string();
        let plain = content(&text);
        if !plain.is_empty() && seen.insert(plain) {
            pool.push(text);
        }
    }
    pool
}

pub fn can_build_sampled(card: &Card, cards: &[Card]) -> bool {
    card.back.len() == 1 && sampled_pool(card, cards).len() >= NUM_OPTIONS - 1
}

/// `sampled_pool` narrowed to the card's fresh AI interchangeability group:
/// only same-deck cards whose cached group label matches the card's own,
/// both labels fingerprint-fresh.
pub fn grouped_pool(
    card: &Card,
    cards: &[Card],
    cache: &crate::augment::AugmentCache,
) -> Vec<String> {
    let Some(own_label) = card.id().and_then(|id| {
        cache
            .group(&id, card.content_fingerprint)
            .map(str::to_string)
    }) else {
        return Vec::new();
    };
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(content(&answer_text(card)));
    let mut pool = Vec::new();
    for other in cards {
        if other.deck_id != card.deck_id || other.token == card.token || other.back.len() != 1 {
            continue;
        }
        let same_group = other
            .id()
            .and_then(|id| {
                cache
                    .group(&id, other.content_fingerprint)
                    .map(str::to_string)
            })
            .is_some_and(|label| label == own_label);
        if !same_group {
            continue;
        }
        let text = answer_text(other).trim().to_string();
        let plain = content(&text);
        if !plain.is_empty() && seen.insert(plain) {
            pool.push(text);
        }
    }
    pool
}

pub fn can_build_grouped(
    card: &Card,
    cards: &[Card],
    cache: &crate::augment::AugmentCache,
) -> bool {
    card.back.len() == 1 && grouped_pool(card, cards, cache).len() >= NUM_OPTIONS - 1
}

pub fn build_sampled(card: &Card, seed: u64, pool: &[String]) -> Option<ChoiceQuestion> {
    if card.back.len() != 1 {
        return None;
    }
    let correct_text = answer_text(card);
    let mut options = sample::sample_distractors(&correct_text, pool, seed, NUM_OPTIONS - 1)?;
    options.push(correct_text.clone());
    let mut rng = Rng::new(seed);
    shuffle(&mut options, &mut rng);
    let correct = options.iter().position(|t| *t == correct_text)?;
    Some(ChoiceQuestion { options, correct })
}

pub fn recognition_question(
    card: &Card,
    seed: u64,
    ai_distractors: Option<&[String]>,
    sampled: Option<&[String]>,
) -> Option<ChoiceQuestion> {
    if card.back.len() != 1 {
        return None;
    }
    if let Some(question) = ai_distractors
        .filter(|d| d.len() >= NUM_OPTIONS - 1)
        .and_then(|ai| build(card, seed, ai))
    {
        return Some(question);
    }
    build_sampled(card, seed, sampled?)
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

    fn stamped(line: usize, back: &str, deck: &str, tok: &str) -> Card {
        let mut c = card(line, back);
        c.deck_id = Arc::from(deck);
        c.token = Some(Arc::from(tok));
        c
    }

    #[test]
    fn sampled_pool_scopes_to_the_deck_and_excludes_siblings_multiline_and_own_content() {
        let target = stamped(1, "alpha", "deck-a", "t1");
        let cards = vec![
            target.clone(),
            stamped(2, "beta", "deck-a", "t2"),
            stamped(3, "gamma", "deck-a", "t1"),
            stamped(4, "delta", "deck-b", "t3"),
            stamped(5, "e\nf", "deck-a", "t4"),
            stamped(6, "**alpha**", "deck-a", "t5"),
            stamped(7, "beta", "deck-a", "t6"),
            stamped(8, "epsilon", "deck-a", "t7"),
        ];
        assert_eq!(
            vec!["beta".to_string(), "epsilon".to_string()],
            sampled_pool(&target, &cards),
            "same-deck other-token single-line answers only, deduped by content, \
             own content excluded even in markup form"
        );
    }

    #[test]
    fn an_unstamped_deck_id_yields_no_pool() {
        let target = card(1, "alpha");
        let cards = vec![target.clone(), card(2, "beta")];
        assert!(sampled_pool(&target, &cards).is_empty());
    }

    #[test]
    fn grouped_pool_scopes_to_the_fresh_group_and_keeps_every_exclusion() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = crate::augment::AugmentCache::open(dir.path().join("a.json"));
        let target = stamped(1, "alpha", "deck-a", "t1");
        let mut sibling = stamped(3, "gamma", "deck-a", "t1");
        sibling.hole = Some(1);
        let cards = vec![
            target.clone(),
            stamped(2, "beta", "deck-a", "t2"),
            sibling,
            stamped(4, "delta", "deck-b", "t3"),
            stamped(5, "e\nf", "deck-a", "t4"),
            stamped(6, "beta", "deck-a", "t5"),
            stamped(7, "epsilon", "deck-a", "t6"),
            stamped(8, "zeta", "deck-a", "t7"),
        ];
        for card in &cards {
            cache.set_group(&card.id().unwrap(), "g0".into(), card.content_fingerprint);
        }
        cache.set_group(
            &cards[7].id().unwrap(),
            "g1".into(),
            cards[7].content_fingerprint,
        );
        assert_eq!(
            vec!["beta".to_string(), "epsilon".to_string()],
            grouped_pool(&target, &cards, &cache),
            "same deck, other token, single line, same fresh group, deduped by content"
        );
        let ungrouped = stamped(9, "eta", "deck-a", "t8");
        assert!(
            grouped_pool(&ungrouped, &cards, &cache).is_empty(),
            "no fresh own label, no pool"
        );
    }

    #[test]
    fn build_sampled_puts_correct_exactly_once_among_four_and_is_deterministic() {
        let target = stamped(1, "alpha", "deck-a", "t1");
        let pool = ai(&["beta", "gamma", "delta", "epsilon"]);
        let q = build_sampled(&target, 42, &pool).unwrap();
        assert_eq!(NUM_OPTIONS, q.options.len());
        assert_eq!(1, q.options.iter().filter(|o| *o == "alpha").count());
        assert_eq!("alpha", q.options[q.correct]);
        let again = build_sampled(&target, 42, &pool).unwrap();
        assert_eq!(q.options, again.options, "equal seed, equal question");
        assert!(
            build_sampled(&target, 42, &ai(&["beta", "gamma"])).is_none(),
            "a two-candidate pool cannot fill three distractor slots"
        );
    }

    #[test]
    fn recognition_question_falls_back_to_sampled_only_when_ai_cannot_build() {
        let c = stamped(1, "alpha", "deck-a", "t1");
        let pool = ai(&["beta", "gamma", "delta"]);
        let from_sampled = recognition_question(&c, 1, None, Some(&pool)).unwrap();
        assert_eq!("alpha", from_sampled.options[from_sampled.correct]);

        let thin_ai = ai(&["w1", "w2"]);
        assert!(
            recognition_question(&c, 1, Some(&thin_ai), Some(&pool)).is_some(),
            "a thin AI cache falls through to the sampled pool"
        );

        let fat_ai = ai(&["w1", "w2", "w3"]);
        let from_ai = recognition_question(&c, 1, Some(&fat_ai), Some(&pool)).unwrap();
        assert!(
            from_ai.options.iter().any(|o| o == "w1"),
            "a buildable AI cache outranks sampling, got {:?}",
            from_ai.options
        );

        assert!(recognition_question(&c, 1, None, None).is_none());
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
        assert_eq!(
            1,
            q.options
                .iter()
                .filter(|option| content(option) == "four")
                .count()
        );
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
        let q = recognition_question(&c, 1, Some(&d), None).unwrap();
        assert_eq!(NUM_OPTIONS, q.options.len());
        assert_eq!("alpha", q.options[q.correct]);
    }

    #[test]
    fn recognition_question_rejects_too_few_ai_distractors() {
        let c = card(1, "alpha");
        assert!(recognition_question(&c, 1, Some(&ai(&["w1", "w2"])), None).is_none());
        assert!(recognition_question(&c, 1, None, None).is_none());
    }

    #[test]
    fn recognition_question_rejects_multi_line_answers() {
        let c = card(1, "line a\nline b");
        let d = ai(&["w1", "w2", "w3"]);
        assert!(recognition_question(&c, 1, Some(&d), None).is_none());
    }

    #[test]
    fn same_appearance_seed_is_stable_but_later_appearances_vary_the_order() {
        let c = card(1, "alpha");
        let d = ai(&["beta", "gamma", "delta"]);
        let id = "q42";

        let first = build(&c, seed_for(id, 100, 1), &d).unwrap();
        let first_again = build(&c, seed_for(id, 100, 1), &d).unwrap();
        assert_eq!(
            first.options, first_again.options,
            "the same appearance must not reshuffle mid-poll"
        );

        // Allow for the rare same-permutation collision across a couple of
        // seeds by checking a handful of later appearances.
        let later_orders_differ = (2..12)
            .map(|appearance| build(&c, seed_for(id, 100, appearance), &d).unwrap())
            .any(|q| q.options != first.options);
        assert!(
            later_orders_differ,
            "no later appearance ever varied the order"
        );
    }
}
