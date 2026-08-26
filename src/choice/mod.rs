pub mod sample;

use std::{
    collections::HashSet,
    hash::{Hash, Hasher},
};

use crate::card::Card;

// One correct option plus three distractors.
pub const NUM_OPTIONS: usize = 4;

#[cfg(feature = "full")]
pub(crate) const NOTE_POSITION_INSTRUCTION: &str = "Never refer to an option by number, letter, \
    or screen position; options are shuffled, so name the claim or mistaken premise itself.";

pub(crate) fn note_names_position(note: &str) -> bool {
    let mut tokens = note
        .split(|character: char| !(character.is_alphanumeric() || character == '#'))
        .filter(|token| !token.is_empty());

    let option = |token: &str| {
        ["option", "options", "choice", "choices"]
            .iter()
            .any(|candidate| token.eq_ignore_ascii_case(candidate))
    };
    let numbered_position = |token: &str| {
        let token = token.strip_prefix('#').unwrap_or(token);
        token.parse::<usize>().is_ok_and(|value| value > 0)
            || ["st", "nd", "rd", "th"].iter().any(|suffix| {
                token
                    .strip_suffix(suffix)
                    .is_some_and(|number| number.parse::<usize>().is_ok_and(|value| value > 0))
            })
            || ["first", "second", "third", "fourth", "fifth", "last"]
                .iter()
                .any(|candidate| token.eq_ignore_ascii_case(candidate))
    };
    let label = |token: &str| {
        ["a", "b", "c", "d", "e"]
            .iter()
            .any(|candidate| token.eq_ignore_ascii_case(candidate))
    };

    let Some(mut previous) = tokens.next() else {
        return false;
    };
    for current in tokens {
        if (option(previous) && (numbered_position(current) || label(current)))
            || (numbered_position(previous) && option(current))
        {
            return true;
        }
        previous = current;
    }
    false
}

#[derive(Debug)]
pub struct ChoiceQuestion {
    pub options: Vec<String>,
    pub correct: usize,
    pub multiple: bool,
    /// Ascending option indices of every correct answer; `[correct]` on a
    /// single-answer question.
    pub correct_set: Vec<usize>,
}

// The whole answer as one option, so a quotation stays (dropping it would
// offer a truncated answer as the correct pick) but loses its marker.
fn answer_text(card: &Card) -> String {
    crate::render::readable_answer_lines(card, crate::card::AnswerSpace::Authored).join("\n")
}

fn content(text: &str, card: &Card) -> String {
    crate::inline::strip_inline_with(text.trim(), &card.definitions)
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
    seen.insert(content(&answer_text(card), card));
    let mut chosen: Vec<String> = Vec::new();
    for option in ai_distractors {
        if chosen.len() == needed {
            break;
        }
        let trimmed = option.trim();
        let content = content(trimmed, card);
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
    Some(ChoiceQuestion {
        options,
        correct,
        multiple: false,
        correct_set: vec![correct],
    })
}

pub fn build_authored(
    card: &Card,
    seed: u64,
    authored_distractors: &[String],
) -> Option<ChoiceQuestion> {
    let correct_text = answer_text(card);
    let mut seen = HashSet::new();
    seen.insert(content(&correct_text, card));
    let mut options = Vec::new();
    for distractor in authored_distractors {
        let trimmed = distractor.trim();
        let content = content(trimmed, card);
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
    Some(ChoiceQuestion {
        options,
        correct,
        multiple: false,
        correct_set: vec![correct],
    })
}

pub fn build_authored_multi(
    card: &Card,
    seed: u64,
    authored_distractors: &[String],
) -> Option<ChoiceQuestion> {
    let mut seen = HashSet::new();
    let mut correct_texts = Vec::new();
    for line in &crate::render::gradeable_answer_lines(card, crate::card::AnswerSpace::Authored) {
        let trimmed = line.trim();
        let content = content(trimmed, card);
        if !content.is_empty() && seen.insert(content) {
            correct_texts.push(trimmed.to_string());
        }
    }
    let mut options = correct_texts.clone();
    let correct_count = options.len();
    for distractor in authored_distractors {
        let trimmed = distractor.trim();
        let content = content(trimmed, card);
        if !content.is_empty() && seen.insert(content) {
            options.push(trimmed.to_string());
        }
    }
    if correct_count == 0 || options.len() == correct_count {
        return None;
    }
    let mut rng = Rng::new(seed);
    shuffle(&mut options, &mut rng);
    let correct_set: Vec<usize> = options
        .iter()
        .enumerate()
        .filter(|(_, option)| correct_texts.contains(option))
        .map(|(index, _)| index)
        .collect();
    let correct = *correct_set.first()?;
    Some(ChoiceQuestion {
        options,
        correct,
        multiple: true,
        correct_set,
    })
}

pub fn can_build(card: &Card, ai_distractors: &[String]) -> bool {
    // Region cards are deliberately excluded, even against stale cached
    // distractors the eligibility gate never minted.
    if card.region.is_some() {
        return false;
    }
    distinct_distractors(card, ai_distractors).len() == NUM_OPTIONS - 1
}

/// A table row card's distractor pool: its own column, i.e. sibling rows of
/// the same container in the same direction (a reversed sibling's answer IS
/// the front column).
pub fn column_pool(card: &Card, deck_cards: &[Card]) -> Vec<String> {
    let (Some(token), Some(row)) = (card.token.as_deref(), card.row.as_deref()) else {
        return Vec::new();
    };
    deck_cards
        .iter()
        .filter(|sibling| {
            sibling.token.as_deref() == Some(token)
                && sibling.reversed == card.reversed
                && sibling.row.as_deref().is_some_and(|r| r != row)
        })
        .map(answer_text)
        .collect()
}

pub fn build_sampled(card: &Card, seed: u64, deck_cards: &[Card]) -> Option<ChoiceQuestion> {
    if card.sampling == Some(false) {
        return None;
    }
    let pool = column_pool(card, deck_cards);
    let sampled = sample::sample_distractors(&answer_text(card), &pool, seed, NUM_OPTIONS - 1)?;
    build(card, seed, &sampled)
}

pub fn can_sample(card: &Card, deck_cards: &[Card]) -> bool {
    if card.region.is_some() {
        return false;
    }
    build_sampled(card, 0, deck_cards).is_some()
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
            Vec::new(),
            line,
        )
    }

    fn ai(distractors: &[&str]) -> Vec<String> {
        distractors.iter().map(|s| s.to_string()).collect()
    }

    fn multi_card(back: &[&str]) -> Card {
        let mut c = card(1, &back.join("\n"));
        c.multiple_choice = true;
        c
    }

    #[test]
    fn a_multi_question_offers_every_correct_line_and_every_distractor() {
        let c = multi_card(&["2", "4"]);
        let q = build_authored_multi(&c, 7, &ai(&["3", "5"])).unwrap();
        assert_eq!(4, q.options.len());
        assert!(q.multiple);
        assert_eq!(2, q.correct_set.len());
        for &i in &q.correct_set {
            assert!(
                ["2", "4"].contains(&q.options[i].as_str()),
                "index {i} must be correct"
            );
        }
        assert_eq!(
            q.correct, q.correct_set[0],
            "correct mirrors the first correct index"
        );
    }

    #[test]
    fn an_atomic_option_keeps_a_supporting_quotation_but_not_its_marker() {
        let c = card(1, "the claim\n> supporting quotation");
        let q = build_authored(&c, 7, &ai(&["a distractor"])).unwrap();

        assert_eq!(
            q.options[q.correct], "the claim\nsupporting quotation",
            "one option is the WHOLE answer, so the quotation stays; dropping it \
             would offer a truncated answer as the correct pick"
        );
        for option in &q.options {
            assert!(
                !option.contains('>'),
                "no option shows authoring syntax: {option:?}"
            );
        }
    }

    #[test]
    fn a_multi_question_does_not_make_supporting_quotation_lines_correct_options() {
        let c = multi_card(&[
            "the claim",
            "> supporting quotation",
            "> continued quotation",
        ]);
        let q = build_authored_multi(&c, 7, &ai(&["distractor one", "distractor two"])).unwrap();

        let correct: Vec<&str> = q
            .correct_set
            .iter()
            .map(|&index| q.options[index].as_str())
            .collect();
        assert_eq!(
            correct,
            ["the claim"],
            "a quotation supports the answer but is not itself a gradeable claim"
        );
    }

    #[test]
    fn a_multi_question_correct_set_is_ascending_and_seed_stable() {
        let c = multi_card(&["alpha", "beta", "gamma"]);
        let d = ai(&["delta", "epsilon"]);
        let q1 = build_authored_multi(&c, 42, &d).unwrap();
        let q2 = build_authored_multi(&c, 42, &d).unwrap();
        assert_eq!(q1.options, q2.options, "same seed, same order");
        assert_eq!(q1.correct_set, q2.correct_set);
        assert!(
            q1.correct_set.windows(2).all(|w| w[0] < w[1]),
            "ascending: {:?}",
            q1.correct_set
        );
    }

    #[test]
    fn a_multi_question_without_a_distractor_does_not_build() {
        let c = multi_card(&["2", "4"]);
        assert!(build_authored_multi(&c, 7, &[]).is_none());
        assert!(
            build_authored_multi(&c, 7, &ai(&["2"])).is_none(),
            "a distractor duplicating a correct option deduplicates away"
        );
    }

    #[test]
    fn a_multi_question_deduplicates_correct_lines_by_content() {
        let c = multi_card(&["**2**", "2", "4"]);
        let q = build_authored_multi(&c, 7, &ai(&["3"])).unwrap();
        assert_eq!(
            3,
            q.options.len(),
            "styled duplicate collapses: {:?}",
            q.options
        );
        assert_eq!(2, q.correct_set.len());
    }

    #[test]
    fn position_dependent_choice_note_references_are_recognized_narrowly() {
        for unsafe_note in [
            "Option 2 reverses the relation.",
            "Choice #3 omits the guard.",
            "The second option is too broad.",
            "Option B confuses identity with sampling.",
        ] {
            assert!(note_names_position(unsafe_note), "{unsafe_note}");
        }
        for safe_note in [
            "The length-limit claim invents a grammar rule.",
            "There are 2 independent reasons.",
            "Option parsing happens before sampling.",
            "Scoping was a choice, not a parser limitation.",
        ] {
            assert!(!note_names_position(safe_note), "{safe_note}");
        }
    }

    fn table_card(token: &str, row: &str, back: &str, reversed: bool) -> Card {
        let mut c = card(1, back);
        c.token = Some(Arc::from(token));
        c.row = Some(Arc::from(row));
        c.reversed = reversed;
        c
    }

    #[test]
    fn the_column_pool_is_same_direction_sibling_rows_of_one_container() {
        let deck = vec![
            table_card("card-t0", "aaaaaa", "alpha", false),
            table_card("card-t0", "bbbbbb", "beta", false),
            table_card("card-t0", "cccccc", "gamma", false),
            table_card("card-t0", "aaaaaa", "front-a", true),
            table_card("card-t0", "bbbbbb", "front-b", true),
            table_card("card-t1", "dddddd", "other-table", false),
            card(9, "rowless"),
        ];
        assert_eq!(
            vec!["beta", "gamma"],
            column_pool(&deck[0], &deck),
            "same container, same direction, other rows only"
        );
        assert_eq!(
            vec!["front-b"],
            column_pool(&deck[3], &deck),
            "the reversed pool is the front column"
        );
        assert!(
            column_pool(&deck[6], &deck).is_empty(),
            "a rowless card never samples"
        );
        assert_eq!(
            vec!["other-table"],
            column_pool(&table_card("card-t1", "eeeeee", "x", false), &deck),
            "pools never cross containers"
        );
    }

    #[test]
    fn build_sampled_needs_three_distinct_column_values() {
        let deck = vec![
            table_card("card-t0", "aaaaaa", "alpha", false),
            table_card("card-t0", "bbbbbb", "beta", false),
            table_card("card-t0", "cccccc", "beta", false),
            table_card("card-t0", "dddddd", "gamma", false),
        ];
        assert!(
            !can_sample(&deck[0], &deck),
            "beta twice leaves a two-value pool"
        );
        assert!(build_sampled(&deck[0], 7, &deck).is_none());

        let mut deck = deck;
        deck.push(table_card("card-t0", "eeeeee", "delta", false));
        assert!(can_sample(&deck[0], &deck));
        let question = build_sampled(&deck[0], 7, &deck).expect("a full pick");
        assert_eq!(NUM_OPTIONS, question.options.len());
        assert_eq!("alpha", question.options[question.correct]);
    }

    #[test]
    fn question_has_four_options_with_correct_exactly_once() {
        let c = card(1, "alpha");
        let d = ai(&["beta", "gamma", "delta", "epsilon", "zeta"]);
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
                .filter(|option| content(option, &c) == "four")
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

    #[test]
    fn a_region_card_takes_no_choice_question_from_any_source() {
        let mut region = card(1, "answer");
        region.region = Some(crate::card::RegionSlot::Single {
            stamp: Some(Arc::from("a1b2c3")),
            hidden: Some("answer".into()),
            line: 3,
        });
        region.authored_distractors = vec!["x".into(), "y".into(), "z".into()];
        assert!(
            !can_build(&region, &ai(&["p", "q", "r"])),
            "cached or authored distractors never build a region choice"
        );
        assert!(!can_sample(&region, &[region.clone()]));
    }

    #[test]
    fn numeric_option_positions_are_one_based() {
        assert!(!note_names_position("Option 0 is impossible."));
        assert!(!note_names_position("The 0th option is impossible."));
        assert!(note_names_position("The 1st option is too broad."));
    }
}
