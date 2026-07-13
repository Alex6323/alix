//! Answer checking for the answer modes.
//!
//! - **Typing**: the answer must be typed character by character with live feedback
//!   ([`TypingValidator`]). Revealing hints marks the card failed.
//! - **TypeLine**: a whole line is typed and submitted, then normalized and compared exactly
//!   ([`grade_typed`]) — no edit-distance tolerance.
//! - **Flip**: the user reveals the answer and grades themselves; no checking happens here.
//! - **Choice**: the user picks the answer out of four options (see the [`choice`](crate::choice)
//!   module).

/// Which answer mode a review session uses.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    clap::ValueEnum,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum Mode {
    /// Reveal the answer and grade yourself (again / good / easy).
    #[default]
    Flip,
    /// Type the answer exactly, character by character.
    Typing,
    /// Type one line at a time, checked in order (Reconstruct + `% reveal: line`).
    #[value(name = "typeline")]
    #[serde(rename = "typeline")]
    TypeLine,
    /// Pick the right answer out of four; the wrong options are sampled from
    /// the other cards of the session.
    Choice,
    /// Reveal the back one line at a time (useful for lyrics or poems), then
    /// grade yourself once the whole card is uncovered.
    #[value(name = "line")]
    LineByLine,
    /// Understanding cards: an open prompt; type your explanation (optional),
    /// reveal the back lines (the key points it should cover), then grade
    /// yourself on whether you covered them.
    Explain,
}

/// The CLI/value name of an answer mode, matching `Mode`'s clap names.
///
/// `pub(crate)`: reused by [`crate::serve`] to report a card's mode in its JSON
/// state.
#[cfg(feature = "full")]
pub(crate) fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Flip => "flip",
        Mode::Typing => "typing",
        Mode::TypeLine => "typeline",
        Mode::Choice => "choice",
        Mode::LineByLine => "line",
        Mode::Explain => "explain",
    }
}

/// How the learner *produces* an answer — orthogonal to [`Mode`] (the grading
/// mode). `Draw` swaps the typed/flip input for a canvas (web only) and
/// self-grades against the card's normal reveal. Set with `% input:` (card
/// override, else deck); absent falls back to `Type`.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    clap::ValueEnum,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum Input {
    /// Type the answer (the default).
    #[default]
    #[value(name = "type")]
    Type,
    /// Draw / handwrite the answer on a canvas (web only), then self-grade.
    #[value(name = "draw")]
    Draw,
}

/// The state of one typed character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Typed {
    /// The character the user typed.
    pub ch: char,
    /// Whether it matches the expected character at its position.
    pub correct: bool,
}

/// Validates one answer line typed character by character.
///
/// The user can type past mistakes, but the line only completes once every
/// position holds the correct character, so wrong characters must be removed
/// with backspace. Asking for a hint reveals upcoming characters; any hint
/// marks the line as failed.
#[derive(Clone, Debug)]
pub struct TypingValidator {
    expected: Vec<char>,
    typed: Vec<Typed>,
    hints_used: usize,
    typos: usize,
    /// How many upcoming characters the current hint reveals; grows by
    /// [`HINT_CHARS`] on each [`hint`](Self::hint) call and resets to 0 when
    /// the user types or deletes a character.
    hint_revealed: usize,
}

/// How many characters a single hint request reveals.
pub const HINT_CHARS: usize = 2;

impl TypingValidator {
    /// Creates a validator for the expected line.
    pub fn new(expected: &str) -> Self {
        Self {
            expected: expected.chars().collect(),
            typed: Vec::new(),
            hints_used: 0,
            typos: 0,
            hint_revealed: 0,
        }
    }

    /// The expected line.
    pub fn expected(&self) -> String {
        self.expected.iter().collect()
    }

    /// The characters typed so far with their correctness.
    pub fn typed(&self) -> &[Typed] {
        &self.typed
    }

    /// Types one character. Returns whether it was correct. Input beyond the
    /// expected length is ignored (and reported as incorrect).
    pub fn type_char(&mut self, ch: char) -> bool {
        self.hint_revealed = 0;
        if self.typed.len() >= self.expected.len() {
            return false;
        }
        let correct = self.expected[self.typed.len()] == ch;
        if !correct {
            self.typos += 1;
        }
        self.typed.push(Typed { ch, correct });
        correct
    }

    /// Removes the last typed character. Returns `true` if one was removed.
    pub fn backspace(&mut self) -> bool {
        self.hint_revealed = 0;
        self.typed.pop().is_some()
    }

    /// Reveals more of the answer: each call uncovers [`HINT_CHARS`] additional
    /// upcoming characters (so repeated requests progressively show the rest of
    /// the line, capped at its length) until the user types or deletes, which
    /// resets the reveal. Any incorrectly typed tail is removed first, so the
    /// hint always follows the correct prefix. Using a hint marks the line
    /// failed.
    pub fn hint(&mut self) -> String {
        self.hints_used += 1;
        if let Some(first_bad) = self.typed.iter().position(|t| !t.correct) {
            self.typed.truncate(first_bad);
        }
        let remaining = self.expected.len() - self.typed.len();
        self.hint_revealed = (self.hint_revealed + HINT_CHARS).min(remaining);
        self.expected
            .iter()
            .skip(self.typed.len())
            .take(self.hint_revealed)
            .collect()
    }

    /// Retargets the validator to a different expected line, re-evaluating the
    /// already-typed characters against it (position by position). Hints and
    /// the typo counter are kept. Used by order-independent multi-line typing,
    /// where the line a row is graded against is chosen as the user types.
    pub fn set_expected(&mut self, expected: &str) {
        self.expected = expected.chars().collect();
        for (i, t) in self.typed.iter_mut().enumerate() {
            t.correct = self.expected.get(i) == Some(&t.ch);
        }
    }

    /// The not-yet-typed remainder of the expected line.
    pub fn remaining(&self) -> String {
        self.expected.iter().skip(self.typed.len()).collect()
    }

    /// `true` once the whole line has been typed correctly.
    pub fn is_complete(&self) -> bool {
        self.typed.len() == self.expected.len() && self.typed.iter().all(|t| t.correct)
    }

    /// `true` if the line was completed without hints.
    pub fn passed(&self) -> bool {
        self.is_complete() && self.hints_used == 0
    }

    /// Number of hints requested.
    pub fn hints_used(&self) -> usize {
        self.hints_used
    }

    /// Number of incorrect characters typed (including later corrected ones).
    pub fn typos(&self) -> usize {
        self.typos
    }
}

/// Zero-risk normalization: what dies here was never a memory signal.
/// Anything that survives normalization and still differs is the learner's
/// call ("typo → Good / wrong → Again"), shown as a diff — never an
/// edit-distance heuristic (affect↔effect is distance 1 and a different word).
pub fn normalize_answer(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(['.', ',', ';', ':', '!', '?'])
        .to_lowercase()
}

/// The result of grading one typed line.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TypedResult {
    /// What the user typed.
    pub input: String,
    /// The expected line.
    pub expected: String,
    /// Whether the normalized input exactly matches the normalized expected line.
    pub passed: bool,
}

/// Grades a typed line: normalizes both sides ([`normalize_answer`]) and
/// compares exactly. No edit-distance tolerance — a typo and a wrong answer
/// both fail here, and it's the learner who decides which was which.
pub fn grade_typed(input: &str, expected: &str) -> TypedResult {
    TypedResult {
        input: input.trim().to_string(),
        expected: expected.to_string(),
        passed: normalize_answer(input) == normalize_answer(expected),
    }
}

/// Grades typed lines against the expected lines **without** regard to order:
/// each input is matched to its closest still-unclaimed expected line (smallest
/// Levenshtein distance on the normalized text — pairing is display logic, not
/// tolerance), so a multi-item answer can be entered in any order. Returns one
/// [`TypedResult`] per input, in input order, each paired with the expected
/// line it was matched to. A single-line answer matches trivially.
pub fn grade_lines_unordered(inputs: &[String], expected: &[String]) -> Vec<TypedResult> {
    let mut claimed = vec![false; expected.len()];
    let mut results = Vec::with_capacity(inputs.len());
    for input in inputs {
        let normalized_input = normalize_answer(input);
        let best = expected
            .iter()
            .enumerate()
            .filter(|(i, _)| !claimed[*i])
            .map(|(i, exp)| {
                (
                    i,
                    strsim::levenshtein(&normalized_input, &normalize_answer(exp)),
                )
            })
            .min_by_key(|(_, distance)| *distance);
        match best {
            Some((i, _)) => {
                claimed[i] = true;
                results.push(grade_typed(input, &expected[i]));
            }
            // More inputs than expected lines: an extra input matches nothing.
            None => results.push(grade_typed(input, "")),
        }
    }
    results
}

/// Grades typed lines against the expected lines **in order**: `inputs[i]` vs
/// `expected[i]` via [`grade_typed`]. An input past the end of `expected` is
/// graded against nothing (an empty expected line, so it fails). This is
/// TypeLine's path (Reconstruct + `% reveal: line`), where each line is answered
/// in sequence and its position carries meaning — unlike [`grade_lines_unordered`].
pub fn grade_lines_ordered(inputs: &[String], expected: &[String]) -> Vec<TypedResult> {
    inputs
        .iter()
        .enumerate()
        .map(|(i, input)| grade_typed(input, expected.get(i).map_or("", String::as_str)))
        .collect()
}

/// Index of the candidate that best continues `typed` as a prefix: the one
/// sharing the longest run of leading characters with `typed` wins; ties go to
/// the shorter candidate, then the earliest. `None` if there are no candidates.
/// Used by order-independent typing to pick which remaining answer line a
/// partially typed row is heading toward, so its characters can be colored
/// against a concrete target.
pub fn best_prefix_match(typed: &str, candidates: &[&str]) -> Option<usize> {
    (0..candidates.len()).min_by_key(|&i| {
        let cand = candidates[i];
        let shared = typed
            .chars()
            .zip(cand.chars())
            .take_while(|(a, b)| a == b)
            .count();
        (std::cmp::Reverse(shared), cand.chars().count(), i)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_parses_its_value_names_and_defaults_to_type() {
        use clap::ValueEnum;
        assert_eq!(Input::Draw, Input::from_str("draw", true).unwrap());
        assert_eq!(Input::Type, Input::from_str("type", true).unwrap());
        assert!(Input::from_str("scribble", true).is_err());
        assert_eq!(Input::Type, Input::default());
    }

    #[test]
    fn typing_correct_line_passes() {
        let mut v = TypingValidator::new("hello");
        for c in "hello".chars() {
            assert!(v.type_char(c));
        }
        assert!(v.is_complete());
        assert!(v.passed());
        assert_eq!(0, v.typos());
    }

    #[test]
    fn typing_incomplete_line_is_not_complete() {
        let mut v = TypingValidator::new("hello");
        v.type_char('h');
        assert!(!v.is_complete());
        assert_eq!("ello", v.remaining());
    }

    #[test]
    fn typing_wrong_char_blocks_completion_until_fixed() {
        let mut v = TypingValidator::new("hello");
        assert!(v.type_char('h'));
        assert!(!v.type_char('3'));
        v.type_char('l');
        v.type_char('l');
        v.type_char('o');
        assert!(!v.is_complete()); // '3' is still wrong at position 1

        // Fix it: backspace everything down to the mistake, retype.
        for _ in 0..4 {
            v.backspace();
        }
        for c in "ello".chars() {
            assert!(v.type_char(c));
        }
        assert!(v.is_complete());
        assert!(v.passed()); // typos don't fail the card, hints do
        assert_eq!(1, v.typos());
    }

    #[test]
    fn typing_input_beyond_length_ignored() {
        let mut v = TypingValidator::new("hi");
        v.type_char('h');
        v.type_char('i');
        assert!(!v.type_char('!'));
        assert!(v.is_complete());
        assert_eq!(2, v.typed().len());
    }

    #[test]
    fn backspace_on_empty_returns_false() {
        let mut v = TypingValidator::new("hi");
        assert!(!v.backspace());
    }

    #[test]
    fn hint_reveals_next_chars_and_fails_the_line() {
        let mut v = TypingValidator::new("hello");
        v.type_char('h');
        assert_eq!("el", v.hint());
        for c in "ello".chars() {
            v.type_char(c);
        }
        assert!(v.is_complete());
        assert!(!v.passed());
        assert_eq!(1, v.hints_used());
    }

    #[test]
    fn repeated_hints_reveal_more_until_full() {
        let mut v = TypingValidator::new("hello");
        v.type_char('h');
        assert_eq!("el", v.hint()); // first request: next two
        assert_eq!("ello", v.hint()); // second: two more
        assert_eq!("ello", v.hint()); // capped at the rest of the line
        // Typing resets the reveal back to two from the new position.
        v.type_char('e');
        assert_eq!("ll", v.hint());
    }

    #[test]
    fn hint_clears_incorrect_tail_first() {
        let mut v = TypingValidator::new("hello");
        v.type_char('h');
        v.type_char('3'); // wrong
        v.type_char('l');
        assert_eq!("el", v.hint());
        // The wrong tail ('3' and everything after) is gone.
        assert_eq!(1, v.typed().len());
        assert_eq!("ello", v.remaining());
    }

    #[test]
    fn hint_at_end_is_empty() {
        let mut v = TypingValidator::new("hi");
        v.type_char('h');
        v.type_char('i');
        assert_eq!("", v.hint());
    }

    #[test]
    fn unicode_is_handled_per_char() {
        let mut v = TypingValidator::new("héllö");
        for c in "héllö".chars() {
            assert!(v.type_char(c));
        }
        assert!(v.passed());
    }

    #[test]
    fn grade_typed_exact_match_passes() {
        let r = grade_typed("hello", "hello");
        assert!(r.passed);
    }

    #[test]
    fn grade_typed_input_is_trimmed() {
        let r = grade_typed("  hello  ", "hello");
        assert!(r.passed);
    }

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unordered_lines_pass_in_any_order() {
        let expected = lines(&["red", "green", "blue"]);
        let inputs = lines(&["blue", "red", "green"]);
        let results = grade_lines_unordered(&inputs, &expected);
        assert!(results.iter().all(|r| r.passed));
        // Each input is paired with the matching expected line.
        assert_eq!("blue", results[0].expected);
        assert_eq!("red", results[1].expected);
        assert_eq!("green", results[2].expected);
    }

    #[test]
    fn unordered_one_wrong_line_maps_to_its_nearest_expected() {
        let expected = lines(&["red", "green", "blue"]);
        // "gren" is closest to "green"; the other two are exact.
        let inputs = lines(&["blue", "gren", "red"]);
        let results = grade_lines_unordered(&inputs, &expected);
        assert!(results[0].passed); // blue
        assert!(!results[1].passed); // gren vs green
        assert_eq!("green", results[1].expected);
        assert!(results[2].passed); // red
    }

    #[test]
    fn unordered_does_not_claim_one_expected_twice() {
        let expected = lines(&["aa", "ab"]);
        // Both inputs are equidistant-ish; each expected line is claimed once.
        let inputs = lines(&["ab", "aa"]);
        let results = grade_lines_unordered(&inputs, &expected);
        let mut matched: Vec<&str> = results.iter().map(|r| r.expected.as_str()).collect();
        matched.sort_unstable();
        assert_eq!(vec!["aa", "ab"], matched);
        assert!(results.iter().all(|r| r.passed));
    }

    #[test]
    fn best_prefix_match_prefers_longest_shared_prefix() {
        let cands = ["green", "grape", "blue"];
        assert_eq!(Some(0), best_prefix_match("gre", &cands));
        assert_eq!(Some(1), best_prefix_match("gra", &cands));
        assert_eq!(Some(2), best_prefix_match("b", &cands));
        // No shared prefix: ties broken toward the shorter, earliest candidate.
        assert_eq!(Some(2), best_prefix_match("x", &cands));
        assert_eq!(None, best_prefix_match("x", &[]));
    }

    #[test]
    fn normalization_forgives_case_whitespace_and_trailing_punctuation() {
        assert!(grade_typed("  Borrow  Checker ", "borrow checker.").passed);
    }

    #[test]
    fn a_one_letter_different_word_is_not_a_typo_and_fails() {
        // affect vs effect: edit distance 1, different word — the learner decides
        // via the override (serve layer), never an edit-distance heuristic here.
        assert!(!grade_typed("affect", "effect").passed);
    }

    #[test]
    fn unordered_lines_pair_each_input_with_its_closest_expected_line() {
        let inputs = vec!["beta".to_string(), "alpha".to_string()];
        let expected = vec!["alpha".to_string(), "beta".to_string()];
        let results = grade_lines_unordered(&inputs, &expected);
        assert!(results.iter().all(|r| r.passed), "order must not matter");
    }

    #[test]
    fn ordered_line_grading_respects_position() {
        let expected = lines(&["red", "green"]);
        // The same lines in swapped order: position-sensitive grading fails both,
        // where unordered matching would have passed them.
        let swapped = grade_lines_ordered(&lines(&["green", "red"]), &expected);
        assert!(!swapped[0].passed, "green vs red");
        assert!(!swapped[1].passed, "red vs green");
        assert_eq!("red", swapped[0].expected); // paired by position, not similarity
        assert_eq!("green", swapped[1].expected);
        // In order, each line matches its position and passes.
        let in_order = grade_lines_ordered(&expected, &expected);
        assert!(in_order.iter().all(|r| r.passed));
        // An extra input beyond the expected lines is graded against nothing.
        let extra = grade_lines_ordered(&lines(&["red", "green", "blue"]), &expected);
        assert!(!extra[2].passed);
        assert_eq!("", extra[2].expected);
    }

    #[test]
    fn set_expected_re_evaluates_typed_chars() {
        let mut v = TypingValidator::new("cat");
        for c in "car".chars() {
            v.type_char(c);
        }
        assert!(!v.is_complete()); // 'r' != 't'
        // Retarget to a line the typed text matches exactly.
        v.set_expected("car");
        assert!(v.is_complete());
        assert!(v.passed());
    }
}
