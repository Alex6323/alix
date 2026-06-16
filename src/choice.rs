//! Multiple-choice questions (`--mode choice`).
//!
//! No deck syntax is involved: the wrong options (distractors) are sampled
//! from the answers of *other cards in the same session*, which are topically
//! coherent and therefore plausible. Distractors are preferentially drawn
//! from answers that look similar to the correct one (years compete with
//! years, commands with commands), with some randomness so the same card
//! doesn't always show the same options.
//!
//! Sub-cards of the same cloze card are never used as distractors — their
//! answers must stay hidden until their own card is reviewed.

use std::collections::HashSet;

use crate::card::Card;

/// Total number of options shown (one correct + three distractors).
pub const NUM_OPTIONS: usize = 4;

/// From how large a pool of the most similar candidates the distractors are
/// sampled.
const SAMPLE_POOL: usize = (NUM_OPTIONS - 1) * 2;

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

/// Crude check whether an answer is "number-like" (years, sizes, ...).
fn is_numeric(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || ",.%/- ".contains(c))
}

/// How unlike two answers are; lower means a more tempting distractor.
/// Mixing number-like and word-like answers makes elimination trivial, so
/// that mismatch dominates the edit distance.
fn dissimilarity(a: &str, b: &str) -> usize {
    let penalty = if is_numeric(a) != is_numeric(b) { 1000 } else { 0 };
    penalty + strsim::levenshtein(a, b)
}

/// Builds a multiple-choice question for `card`, sampling distractors from
/// `pool` (the session's cards). Returns `None` if the pool doesn't contain
/// enough distinct answers — the caller should fall back to another mode.
pub fn build(card: &Card, pool: &[Card], seed: u64) -> Option<ChoiceQuestion> {
    let correct_text = answer_text(card);

    let mut seen = HashSet::new();
    let mut candidates: Vec<String> = Vec::new();
    for other in pool {
        // Skip the card itself and its cloze siblings (same file + line):
        // sibling answers must not be revealed as options.
        if other.subject == card.subject && other.line == card.line {
            continue;
        }
        let text = answer_text(other);
        if text == correct_text || text.trim().is_empty() {
            continue;
        }
        if seen.insert(text.clone()) {
            candidates.push(text);
        }
    }

    let needed = NUM_OPTIONS - 1;
    if candidates.len() < needed {
        return None;
    }

    // Keep the most similar candidates, then sample among them.
    candidates.sort_by_key(|text| dissimilarity(&correct_text, text));
    candidates.truncate(SAMPLE_POOL);

    let mut rng = Rng::new(seed);
    shuffle(&mut candidates, &mut rng);
    candidates.truncate(needed);

    let mut options = candidates;
    options.push(correct_text.clone());
    shuffle(&mut options, &mut rng);
    let correct = options.iter().position(|t| *t == correct_text).unwrap();

    Some(ChoiceQuestion { options, correct })
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
    use super::*;
    use std::sync::Arc;

    fn card(line: usize, back: &str) -> Card {
        Card::plain(
            Arc::from("deck.txt"),
            format!("front {line}"),
            back.split('\n').map(String::from).collect(),
            None,
            line,
        )
    }

    fn pool(backs: &[&str]) -> Vec<Card> {
        backs.iter().enumerate().map(|(i, b)| card(i + 1, b)).collect()
    }

    #[test]
    fn question_has_four_options_with_correct_exactly_once() {
        let cards = pool(&["alpha", "beta", "gamma", "delta", "epsilon"]);
        let q = build(&cards[0], &cards, 42).unwrap();
        assert_eq!(NUM_OPTIONS, q.options.len());
        assert_eq!(1, q.options.iter().filter(|o| *o == "alpha").count());
        assert_eq!("alpha", q.options[q.correct]);
    }

    #[test]
    fn too_small_pool_yields_none() {
        let cards = pool(&["alpha", "beta", "gamma"]);
        assert!(build(&cards[0], &cards, 42).is_none());
    }

    #[test]
    fn duplicate_answers_count_once() {
        // Three distinct distractor texts are required; duplicates of the
        // answer or of each other must not pad the pool.
        let cards = pool(&["alpha", "beta", "beta", "alpha", "gamma"]);
        assert!(build(&cards[0], &cards, 42).is_none());
    }

    #[test]
    fn cloze_siblings_are_never_distractors() {
        // Cards 1..=3 share line 1 (cloze sub-cards of one source card).
        let mut cards = Vec::new();
        for back in ["hole one", "hole two", "hole three"] {
            cards.push(card(1, back));
        }
        for (i, back) in ["w", "x", "y", "z"].iter().enumerate() {
            cards.push(card(10 + i, back));
        }
        for seed in 0..20 {
            let q = build(&cards[0], &cards, seed).unwrap();
            assert!(!q.options.contains(&"hole two".to_string()));
            assert!(!q.options.contains(&"hole three".to_string()));
        }
    }

    #[test]
    fn numeric_answers_get_numeric_distractors() {
        let cards = pool(&[
            "1158",
            "1240",
            "1632",
            "1806",
            "1918",
            "1972",
            "2005",
            "Heinrich der Löwe",
            "Marienplatz",
            "Isar",
        ]);
        // Plenty of numeric candidates exist, so no word-like answer should
        // ever appear as a distractor for "1158".
        for seed in 0..20 {
            let q = build(&cards[0], &cards, seed).unwrap();
            for option in &q.options {
                assert!(is_numeric(option), "word distractor {option:?} for a year");
            }
        }
    }

    #[test]
    fn same_seed_same_question() {
        let cards = pool(&["alpha", "beta", "gamma", "delta", "epsilon", "zeta"]);
        let a = build(&cards[0], &cards, 7).unwrap();
        let b = build(&cards[0], &cards, 7).unwrap();
        assert_eq!(a.options, b.options);
        assert_eq!(a.correct, b.correct);
    }

    #[test]
    fn different_seeds_vary_the_options() {
        let cards =
            pool(&["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"]);
        let questions: HashSet<Vec<String>> =
            (0..10).map(|seed| build(&cards[0], &cards, seed).unwrap().options).collect();
        assert!(questions.len() > 1, "options never varied across seeds");
    }

    #[test]
    fn multi_line_answers_become_one_option() {
        let cards = pool(&["line a\nline b", "x", "y", "z", "w"]);
        let q = build(&cards[0], &cards, 1).unwrap();
        assert_eq!("line a\nline b", q.options[q.correct]);
    }
}
