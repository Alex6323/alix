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

use std::{collections::HashSet, sync::Arc};

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
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_digit() || ",.%/- ".contains(c))
}

/// A coarse "shape" of a number-like answer: its digit count plus the distinct
/// non-digit symbols it uses. A 4-digit year shares a shape with other 4-digit
/// years (so they compete as distractors), but not with a 2-digit count or a
/// "1,5" ratio — so a decimal never sneaks in as an option for a year. `None`
/// for words, which then compete only with other words.
fn numeric_shape(s: &str) -> Option<(usize, String)> {
    if !is_numeric(s) {
        return None;
    }
    let digits = s.chars().filter(char::is_ascii_digit).count();
    let mut seps: Vec<char> = s.chars().filter(|c| !c.is_ascii_digit()).collect();
    seps.sort_unstable();
    seps.dedup();
    Some((digits, seps.into_iter().collect()))
}

/// How unlike two answers are; lower means a more tempting distractor. Answers
/// of a different *shape* — a number vs a word, or a 4-digit year vs a "1,5"
/// ratio — are pushed far apart so elimination stays non-trivial; within a
/// shape, edit distance ranks them.
fn dissimilarity(a: &str, b: &str) -> usize {
    let penalty = if numeric_shape(a) != numeric_shape(b) {
        1000
    } else {
        0
    };
    penalty + strsim::levenshtein(a, b)
}

/// Builds a multiple-choice question for `card`. Distractors come first from
/// `ai_distractors` (Claude-generated, when AI augmentation is on), then are
/// topped up by sampling `pool` (the session's other cards) when fewer than
/// needed were supplied. Returns `None` if neither source yields enough distinct
/// wrong answers — the caller should fall back to another mode.
///
/// With `ai_distractors == None` this is the pure offline sampler, unchanged.
pub fn build(
    card: &Card,
    pool: &[Card],
    seed: u64,
    ai_distractors: Option<&[String]>,
) -> Option<ChoiceQuestion> {
    let correct_text = answer_text(card);
    let needed = NUM_OPTIONS - 1;
    let mut rng = Rng::new(seed);

    // The correct answer plus everything already chosen, so neither a sampled nor
    // an AI option can duplicate them.
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(correct_text.clone());

    // AI distractors take precedence; validate against the answer and dedup.
    let mut chosen: Vec<String> = Vec::new();
    for option in ai_distractors.unwrap_or(&[]) {
        if chosen.len() == needed {
            break;
        }
        let trimmed = option.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
            chosen.push(trimmed.to_string());
        }
    }

    // Top up from offline sampling when AI didn't supply enough — preferring the
    // most similar answers, with some randomness so the options vary by seed.
    if chosen.len() < needed {
        let mut candidates = offline_candidates(card, pool, &seen);
        candidates.sort_by_key(|text| dissimilarity(&correct_text, text));
        candidates.truncate(SAMPLE_POOL);
        shuffle(&mut candidates, &mut rng);
        for candidate in candidates {
            if chosen.len() == needed {
                break;
            }
            chosen.push(candidate);
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

/// Builds a recognition question for a card's *first encounter* (acquire), under
/// the strict bar that makes a guess worth the trouble: an **atomic** card (a
/// single-line answer) whose deck supplies a full set of *AI* distractors
/// (`alix deck augment --target choices`). Offline-sampled distractors are
/// deliberately not enough here — a junk multiple-choice is worse than an honest
/// reveal — so a card without cached AI distractors (or with a multi-line answer)
/// returns `None`, and the frontend shows the recall-then-reveal acquire instead.
pub fn recognition_question(
    card: &Card,
    pool: &[Card],
    seed: u64,
    ai_distractors: Option<&[String]>,
) -> Option<ChoiceQuestion> {
    if card.back.len() != 1 {
        return None;
    }
    // A full set of AI distractors, or nothing — never top up from offline sampling
    // for a first encounter.
    let ai = ai_distractors.filter(|d| d.len() >= NUM_OPTIONS - 1)?;
    build(card, pool, seed, Some(ai))
}

/// Builds a Recognize-level question for a line-reveal card: pick the next
/// line. Distractors prefer the card's OWN other lines first — confusable by
/// construction, since they belong to the same ordered answer — then `ai`,
/// via [`build`]. There is no cross-card pool tier: a line card's own lines are
/// already the natural distractors. Returns `None` when `next` is out of range
/// or fewer than `NUM_OPTIONS` distinct options are available.
pub fn line_question(
    card: &Card,
    next: usize,
    seed: u64,
    ai: Option<&[String]>,
) -> Option<ChoiceQuestion> {
    let correct = card.back.get(next)?.clone();
    let target = Card::plain(
        Arc::clone(&card.subject),
        card.front.clone(),
        vec![correct],
        None,
        card.line,
    );

    let mut preferred: Vec<String> = card
        .back
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != next)
        .map(|(_, line)| line.clone())
        .collect();
    preferred.extend(ai.unwrap_or(&[]).iter().cloned());

    build(&target, &[], seed, Some(&preferred))
}

/// Distractor candidates sampled from `pool`: every other card's answer, minus
/// the card's own cloze siblings (same file + line), empty answers, and anything
/// in `exclude` (the correct answer plus any already-chosen options).
fn offline_candidates(card: &Card, pool: &[Card], exclude: &HashSet<String>) -> Vec<String> {
    let mut seen = exclude.clone();
    let mut candidates = Vec::new();
    for other in pool {
        // Sibling answers (same source line) must not be revealed as options.
        if other.subject == card.subject && other.line == card.line {
            continue;
        }
        let text = answer_text(other);
        if text.trim().is_empty() {
            continue;
        }
        if seen.insert(text.clone()) {
            candidates.push(text);
        }
    }
    candidates
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

    fn pool(backs: &[&str]) -> Vec<Card> {
        backs
            .iter()
            .enumerate()
            .map(|(i, b)| card(i + 1, b))
            .collect()
    }

    #[test]
    fn question_has_four_options_with_correct_exactly_once() {
        let cards = pool(&["alpha", "beta", "gamma", "delta", "epsilon"]);
        let q = build(&cards[0], &cards, 42, None).unwrap();
        assert_eq!(NUM_OPTIONS, q.options.len());
        assert_eq!(1, q.options.iter().filter(|o| *o == "alpha").count());
        assert_eq!("alpha", q.options[q.correct]);
    }

    #[test]
    fn too_small_pool_yields_none() {
        let cards = pool(&["alpha", "beta", "gamma"]);
        assert!(build(&cards[0], &cards, 42, None).is_none());
    }

    #[test]
    fn duplicate_answers_count_once() {
        // Three distinct distractor texts are required; duplicates of the
        // answer or of each other must not pad the pool.
        let cards = pool(&["alpha", "beta", "beta", "alpha", "gamma"]);
        assert!(build(&cards[0], &cards, 42, None).is_none());
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
            let q = build(&cards[0], &cards, seed, None).unwrap();
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
            let q = build(&cards[0], &cards, seed, None).unwrap();
            for option in &q.options {
                assert!(is_numeric(option), "word distractor {option:?} for a year");
            }
        }
    }

    #[test]
    fn a_decimal_never_competes_with_years() {
        // "1,5" is number-like but a different shape than a 4-digit year, so it
        // must not be sampled as a distractor for one while real years exist.
        let cards = pool(&[
            "1589", "1158", "1789", "1638", "1568", "1328", "1807", "1,5",
        ]);
        for seed in 0..30 {
            let q = build(&cards[0], &cards, seed, None).unwrap();
            assert!(
                !q.options.contains(&"1,5".to_string()),
                "1,5 sampled as a year distractor (seed {seed})"
            );
        }
    }

    #[test]
    fn same_seed_same_question() {
        let cards = pool(&["alpha", "beta", "gamma", "delta", "epsilon", "zeta"]);
        let a = build(&cards[0], &cards, 7, None).unwrap();
        let b = build(&cards[0], &cards, 7, None).unwrap();
        assert_eq!(a.options, b.options);
        assert_eq!(a.correct, b.correct);
    }

    #[test]
    fn different_seeds_vary_the_options() {
        let cards = pool(&[
            "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta",
        ]);
        let questions: HashSet<Vec<String>> = (0..10)
            .map(|seed| build(&cards[0], &cards, seed, None).unwrap().options)
            .collect();
        assert!(questions.len() > 1, "options never varied across seeds");
    }

    #[test]
    fn multi_line_answers_become_one_option() {
        let cards = pool(&["line a\nline b", "x", "y", "z", "w"]);
        let q = build(&cards[0], &cards, 1, None).unwrap();
        assert_eq!("line a\nline b", q.options[q.correct]);
    }

    #[test]
    fn ai_distractors_are_used_even_when_the_pool_is_too_thin() {
        // Only the card itself is in the pool, so offline sampling can't build a
        // question — but three AI distractors can.
        let cards = pool(&["alpha"]);
        let ai = [
            "wrong one".to_string(),
            "wrong two".to_string(),
            "wrong three".to_string(),
        ];
        let q = build(&cards[0], &cards, 1, Some(&ai)).unwrap();
        assert_eq!(NUM_OPTIONS, q.options.len());
        assert_eq!("alpha", q.options[q.correct]);
        assert!(q.options.contains(&"wrong one".to_string()));
    }

    #[test]
    fn ai_distractors_are_topped_up_from_the_pool_when_short() {
        // One AI distractor plus a healthy pool: the AI option is kept and the
        // remaining slots are sampled offline.
        let cards = pool(&["alpha", "beta", "gamma", "delta", "epsilon"]);
        let ai = ["ai wrong".to_string()];
        let q = build(&cards[0], &cards, 3, Some(&ai)).unwrap();
        assert_eq!(NUM_OPTIONS, q.options.len());
        assert!(q.options.contains(&"ai wrong".to_string()));
        assert_eq!("alpha", q.options[q.correct]);
    }

    #[test]
    fn an_ai_distractor_equal_to_the_answer_is_dropped() {
        let cards = pool(&["alpha", "beta", "gamma", "delta", "epsilon"]);
        // "alpha" is the correct answer and must never appear as a second option.
        let ai = ["alpha".to_string(), "ai wrong".to_string()];
        for seed in 0..10 {
            let q = build(&cards[0], &cards, seed, Some(&ai)).unwrap();
            assert_eq!(1, q.options.iter().filter(|o| *o == "alpha").count());
        }
    }

    #[test]
    fn recognition_question_needs_atomic_answer_and_full_ai_distractors() {
        let cards = pool(&["alpha", "beta", "gamma", "delta", "epsilon"]);
        let ai = ["w1".to_string(), "w2".to_string(), "w3".to_string()];
        let q = recognition_question(&cards[0], &cards, 1, Some(&ai)).unwrap();
        assert_eq!(NUM_OPTIONS, q.options.len());
        assert_eq!("alpha", q.options[q.correct]);
    }

    #[test]
    fn recognition_question_rejects_too_few_ai_distractors() {
        // A short AI set must NOT be rescued by offline sampling — a junk MC is
        // worse than an honest reveal, so it falls back to recall-acquire (None).
        let cards = pool(&["alpha", "beta", "gamma", "delta", "epsilon"]);
        let ai = ["w1".to_string(), "w2".to_string()];
        assert!(recognition_question(&cards[0], &cards, 1, Some(&ai)).is_none());
        assert!(recognition_question(&cards[0], &cards, 1, None).is_none());
    }

    #[test]
    fn recognition_question_rejects_multi_line_answers() {
        // An open / multi-line answer can't be a meaningful pick-one.
        let cards = pool(&["line a\nline b", "x", "y", "z", "w"]);
        let ai = ["w1".to_string(), "w2".to_string(), "w3".to_string()];
        assert!(recognition_question(&cards[0], &cards, 1, Some(&ai)).is_none());
    }

    #[test]
    fn line_question_prefers_same_card_lines_as_distractors() {
        let target = card(1, "line a\nline b\nline c\nline d");
        // `line_question` takes no pool — the card's OWN other lines are the
        // only distractor source, and win by construction.
        let q = line_question(&target, 0, 1, None).unwrap();
        assert_eq!(NUM_OPTIONS, q.options.len());
        assert_eq!("line a", q.options[q.correct]);
        for opt in &q.options {
            if opt != "line a" {
                assert!(
                    ["line b", "line c", "line d"].contains(&opt.as_str()),
                    "distractor {opt:?} did not come from the card's own lines"
                );
            }
        }
    }

    #[test]
    fn line_question_falls_back_to_none_below_four_options() {
        // A single-line card has no sibling lines and no cross-card pool tier, so
        // it can't reach four options — the caller falls back to attempt→reveal.
        let line_card = card(1, "only one line");
        assert!(line_question(&line_card, 0, 1, None).is_none());
    }
}
